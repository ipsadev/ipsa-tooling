use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
};

use anyhow::{bail, Context, Result};

use crate::logger;

fn log_cmd(program: &str, args: &[&str]) {
    crate::logger::detail(&format!("$ {program} {}", shown(args)));
}

fn shown(args: &[&str]) -> String {
    let redacted: Vec<&str> = args
        .iter()
        .map(|argument| {
            if is_stellar_secret(argument) {
                "S…(secret)"
            } else {
                argument
            }
        })
        .collect();

    redacted.join(" ")
}

fn is_stellar_secret(argument: &str) -> bool {
    argument.len() == 56
        && argument.starts_with('S')
        && argument
            .chars()
            .all(|character| character.is_ascii_uppercase() || ('2'..='7').contains(&character))
}

pub fn command(root: &Path, program: &str, args: &[&str]) -> Result<()> {
    match logger::current_bar() {
        Some(bar) => command_streaming(root, program, args, &bar),
        _ => command_inherit(root, program, args),
    }
}

fn command_inherit(root: &Path, program: &str, args: &[&str]) -> Result<()> {
    log_cmd(program, args);

    let status = Command::new(program)
        .args(args)
        .current_dir(root)
        .status()
        .with_context(|| format!("failed to spawn {program}"))?;
    if !status.success() {
        bail!("{program} {} exited with {status}", shown(args));
    }
    Ok(())
}

fn command_streaming(
    root: &Path,
    program: &str,
    args: &[&str],
    bar: &indicatif::ProgressBar,
) -> Result<()> {
    log_cmd(program, args);

    let mut child = Command::new(program)
        .args(args)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {program}"))?;

    let stdout = child.stdout.take().context("child stdout unavailable")?;
    let stderr = child.stderr.take().context("child stderr unavailable")?;

    let (tx, rx) = mpsc::channel::<String>();
    let tx_err = tx.clone();

    let out_handle = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let err_handle = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx_err.send(line);
        }
    });

    for line in rx {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            bar.set_message(trimmed.to_string());
        }
    }

    let _ = out_handle.join();
    let _ = err_handle.join();

    let status = child.wait().with_context(|| format!("{program} failed"))?;
    if !status.success() {
        bail!("{program} {} exited with {status}", shown(args));
    }
    Ok(())
}

pub fn piped(root: &Path, program: &str, args: &[&str], input: &str) -> Result<()> {
    log_cmd(program, args);

    let mut child = Command::new(program)
        .args(args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {program}"))?;

    child
        .stdin
        .as_mut()
        .context("child stdin unavailable")?
        .write_all(input.as_bytes())
        .context("failed to write to child stdin")?;

    let status = child.wait().with_context(|| format!("{program} failed"))?;

    if !status.success() {
        bail!("{program} {} exited with {status}", shown(args));
    }

    Ok(())
}

pub fn capture(root: &Path, program: &str, args: &[&str]) -> Result<String> {
    log_cmd(program, args);

    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("failed to spawn {program}"))?;

    if !output.status.success() {
        bail!("{program} {} exited with {}", shown(args), output.status);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn capture_quiet(root: &Path, program: &str, args: &[&str]) -> Result<String> {
    log_cmd(program, args);

    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to spawn {program}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);

        bail!(
            "{program} {} exited with {}:\n{stderr}",
            shown(args),
            output.status
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn capture_all(root: &Path, program: &str, args: &[&str]) -> Result<String> {
    log_cmd(program, args);

    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to spawn {program}"))?;

    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));

    if !output.status.success() {
        bail!(
            "{program} {} exited with {}:\n{combined}",
            shown(args),
            output.status
        );
    }

    Ok(combined)
}

pub fn capture_with_input(
    root: &Path,
    program: &str,
    args: &[&str],
    input: &str,
) -> Result<String> {
    log_cmd(program, args);

    let mut child = Command::new(program)
        .args(args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {program}"))?;

    child
        .stdin
        .take()
        .context("child stdin unavailable")?
        .write_all(input.as_bytes())
        .context("failed to write to child stdin")?;

    let output = child
        .wait_with_output()
        .with_context(|| format!("{program} failed"))?;

    if !output.status.success() {
        bail!(
            "{program} {} exited with {}:\n{}",
            shown(args),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn has(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
