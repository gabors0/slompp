//! Handing things to the desktop: files to an editor, URLs to the browser,
//! folders to the file manager and a shell to a terminal.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, bail};

/// Opens a file or folder with `editor` (a command line, e.g. `code -n`) or
/// the desktop's default application.
pub fn open_path(path: &Path, editor: Option<&str>) -> anyhow::Result<()> {
    if !path.exists() {
        bail!("{} doesn't exist yet", path.display());
    }
    match editor.map(str::trim).filter(|e| !e.is_empty()) {
        Some(editor) => {
            let mut words = editor.split_whitespace();
            let program = words.next().unwrap_or(editor);
            let mut cmd = Command::new(program);
            cmd.args(words).arg(path);
            launch(cmd)
        }
        None => open_default(path.as_os_str()),
    }
}

pub fn open_url(url: &str) -> anyhow::Result<()> {
    open_default(OsStr::new(url))
}

fn open_default(target: &OsStr) -> anyhow::Result<()> {
    let cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(target);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]).arg(target);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(target);
        c
    };
    launch(cmd)
}

/// Terminals tried in order when none is configured.
#[cfg(all(unix, not(target_os = "macos")))]
const TERMINALS: &[&str] = &[
    "x-terminal-emulator",
    "konsole",
    "gnome-terminal",
    "kgx",
    "ptyxis",
    "kitty",
    "alacritty",
    "wezterm",
    "foot",
    "xfce4-terminal",
    "tilix",
    "xterm",
];

/// Opens a terminal in `dir` with `env` set (the stack's PATH and MariaDB
/// socket), like XAMPP's Shell button.
pub fn open_terminal(
    dir: &Path,
    env: &[(String, String)],
    terminal: Option<&str>,
) -> anyhow::Result<()> {
    let configured = terminal.map(str::trim).filter(|t| !t.is_empty());
    let mut cmd = if let Some(terminal) = configured {
        let mut words = terminal.split_whitespace();
        let mut c = Command::new(words.next().unwrap_or(terminal));
        c.args(words);
        c
    } else {
        default_terminal()?
    };
    cmd.current_dir(dir);
    for (k, v) in env {
        cmd.env(k, v);
    }
    launch(cmd)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn default_terminal() -> anyhow::Result<Command> {
    if let Ok(term) = std::env::var("TERMINAL")
        && crate::toolchain::find_program(&[term.as_str()]).is_some()
    {
        return Ok(Command::new(term));
    }
    let found = TERMINALS
        .iter()
        .find(|t| crate::toolchain::find_program(&[t]).is_some())
        .context("no terminal found; choose one in Settings")?;
    Ok(Command::new(found))
}

#[cfg(target_os = "macos")]
fn default_terminal() -> anyhow::Result<Command> {
    let mut c = Command::new("open");
    c.args(["-a", "Terminal", "."]);
    Ok(c)
}

#[cfg(windows)]
fn default_terminal() -> anyhow::Result<Command> {
    let mut c = Command::new("cmd");
    c.args(["/C", "start", "cmd"]);
    Ok(c)
}

/// Starts a program without waiting for it, reaping it in the background.
fn launch(mut cmd: Command) -> anyhow::Result<()> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd
        .spawn()
        .with_context(|| format!("couldn't start {program}"))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}
