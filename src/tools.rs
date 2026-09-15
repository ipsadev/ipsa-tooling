use std::path::Path;

use anyhow::Result;

use crate::run;

pub mod stellar {
    use super::*;

    pub fn command(root: &Path, args: &[&str]) -> Result<()> {
        run::command(root, "stellar", args)
    }

    pub fn capture(root: &Path, args: &[&str]) -> Result<String> {
        run::capture(root, "stellar", args)
    }

    pub fn capture_quiet(root: &Path, args: &[&str]) -> Result<String> {
        run::capture_quiet(root, "stellar", args)
    }

    pub fn capture_all(root: &Path, args: &[&str]) -> Result<String> {
        run::capture_all(root, "stellar", args)
    }
}

pub mod gaiad {
    use super::*;

    pub fn command(root: &Path, args: &[&str]) -> Result<()> {
        run::command(root, "gaiad", args)
    }

    pub fn capture_quiet(root: &Path, args: &[&str]) -> Result<String> {
        run::capture_quiet(root, "gaiad", args)
    }

    pub fn piped(root: &Path, args: &[&str], input: &str) -> Result<()> {
        run::piped(root, "gaiad", args, input)
    }
}

pub mod git {
    use super::*;

    pub fn command(root: &Path, args: &[&str]) -> Result<()> {
        run::command(root, "git", args)
    }
}

pub mod docker {
    use super::*;

    const COMPOSE_PROFILES: [&str; 8] = [
        "--profile",
        "local",
        "--profile",
        "ibcv2",
        "--profile",
        "watcher",
        "--profile",
        "verifier",
    ];

    pub fn command(root: &Path, args: &[&str]) -> Result<()> {
        run::command(root, "docker", args)
    }

    pub fn capture_all(root: &Path, args: &[&str]) -> Result<String> {
        run::capture_all(root, "docker", args)
    }

    pub fn piped(root: &Path, args: &[&str], input: &str) -> Result<()> {
        run::piped(root, "docker", args, input)
    }

    pub fn compose_argv(extra: &[&str]) -> Vec<String> {
        let mut argv = vec!["compose".to_string()];
        argv.extend(COMPOSE_PROFILES.iter().map(|s| (*s).to_string()));
        argv.extend(extra.iter().map(|s| (*s).to_string()));

        argv
    }

    pub fn compose(root: &Path, extra: &[&str]) -> Result<()> {
        let argv = compose_argv(extra);
        let refs: Vec<&str> = argv.iter().map(String::as_str).collect();

        command(root, &refs)
    }
}

#[cfg(test)]
mod tests {
    use super::docker;

    #[test]
    fn compose_argv_prepends_profiles() {
        assert_eq!(
            docker::compose_argv(&["up", "-d", "cosmos"]),
            vec![
                "compose",
                "--profile",
                "local",
                "--profile",
                "ibcv2",
                "--profile",
                "watcher",
                "--profile",
                "verifier",
                "up",
                "-d",
                "cosmos"
            ]
        );
    }

    #[test]
    fn compose_argv_with_no_extra_is_just_the_prefix() {
        assert_eq!(
            docker::compose_argv(&[]),
            vec![
                "compose",
                "--profile",
                "local",
                "--profile",
                "ibcv2",
                "--profile",
                "watcher",
                "--profile",
                "verifier"
            ]
        );
    }
}
