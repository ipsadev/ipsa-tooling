use std::{
    io::IsTerminal,
    sync::{Mutex, OnceLock},
    time::Instant,
};

use indicatif::ProgressBar;

static START: OnceLock<Instant> = OnceLock::new();

static CURRENT: OnceLock<Mutex<Option<ProgressBar>>> = OnceLock::new();

fn start() -> Instant {
    *START.get_or_init(Instant::now)
}

fn current() -> &'static Mutex<Option<ProgressBar>> {
    CURRENT.get_or_init(|| Mutex::new(None))
}

pub fn init() {
    let _ = start();
}

fn tty() -> bool {
    std::io::stdout().is_terminal()
}

fn paint(code: &str, text: &str) -> String {
    if tty() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub fn current_bar() -> Option<ProgressBar> {
    current()
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())
}

fn emit(line: String) {
    match current_bar() {
        Some(bar) => bar.println(line),
        None => println!("{line}"),
    }
}

pub fn banner(text: &str) {
    emit(format!("\n{}", paint("1;36", &format!("=== {text} ==="))));
}

pub fn step(text: &str) {
    emit(format!("\n{} {}", paint("1;34", "▶"), paint("1", text)));
}

pub fn ok(text: &str) {
    emit(format!("  {} {}", paint("1;32", "✓"), text));
}

pub fn warn(text: &str) {
    emit(format!("  {} {}", paint("1;33", "!"), text));
}

pub fn fail(text: &str) {
    emit(format!("  {} {}", paint("1;31", "✗"), text));
}

pub fn detail(text: &str) {
    emit(format!("    {}", paint("2", text)));
}

pub fn hint(text: &str) {
    emit(format!("\n{} {}", paint("1;35", "→"), text));
}

pub fn status_line(label: &str, up: bool, note: &str) {
    let dot = if up {
        paint("1;32", "●")
    } else {
        paint("1;31", "●")
    };
    let state = if up {
        paint("32", "up")
    } else {
        paint("31", "down")
    };
    emit(format!(
        "  {dot} {label:<16} {state:<6} {}",
        paint("2", note)
    ));
}
