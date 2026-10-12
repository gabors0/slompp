//! Handing things to the desktop: files to an editor, URLs to the browser,
//! folders to the file manager and a shell to a terminal.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, bail};

/// Opens a file or folder with `editor` (a command line, e.g. `code -n`; on
/// macOS also an application's name, e.g. `Visual Studio Code`) or the
/// desktop's default application.
pub fn open_path(path: &Path, editor: Option<&str>) -> anyhow::Result<()> {
    if !path.exists() {
        bail!("{} doesn't exist yet", path.display());
    }
    match editor.map(str::trim).filter(|e| !e.is_empty()) {
        Some(editor) => {
            let mut cmd = command_or_app(editor);
            cmd.arg(path);
            launch(cmd)
        }
        // `.conf`, `.cnf` and friends belong to no application on macOS, so
        // name the default text editor.
        None if cfg!(target_os = "macos") && path.is_file() => {
            let mut cmd = Command::new("open");
            cmd.arg("-t").arg(path);
            launch(cmd)
        }
        None => open_default(path.as_os_str()),
    }
}

/// The command a setting names. On macOS, when its first word isn't a
/// program, the whole setting is an application's name for `open -a`.
fn command_or_app(setting: &str) -> Command {
    let mut words = setting.split_whitespace();
    let program = words.next().unwrap_or(setting);
    if cfg!(target_os = "macos")
        && !program.contains('/')
        && crate::toolchain::find_program(&[program]).is_none()
    {
        let mut cmd = Command::new("open");
        cmd.args(["-a", setting]);
        return cmd;
    }
    let mut cmd = Command::new(program);
    cmd.args(words);
    cmd
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
#[cfg(not(target_os = "macos"))]
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

/// Opens a terminal in `dir` with `env` set (the stack's PATH and MariaDB
/// socket), like XAMPP's Shell button.
///
/// macOS terminals are applications started through `open`, which passes on
/// neither our environment nor a working directory. They do run a `.command`
/// script handed to them, so the shell is set up by one in `dir/run`.
/// `terminal` is an application's name (`iTerm`, `Ghostty`; Terminal when
/// unset) or, as elsewhere, a command such as `kitty` or `wezterm start`.
#[cfg(target_os = "macos")]
pub fn open_terminal(
    dir: &Path,
    env: &[(String, String)],
    terminal: Option<&str>,
) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let terminal = terminal
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("Terminal");
    let mut cmd = command_or_app(terminal);
    if cmd.get_program() != "open" {
        cmd.current_dir(dir);
        for (k, v) in env {
            cmd.env(k, v);
        }
        return launch(cmd);
    }

    let script = dir.join("run").join("shell.command");
    if let Some(run) = script.parent() {
        std::fs::create_dir_all(run)?;
    }
    std::fs::write(&script, shell_script(dir, env))
        .with_context(|| format!("writing {}", script.display()))?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    cmd.arg(&script);
    launch(cmd)
}

/// A script that becomes an interactive shell in `dir` with `env` set. Not a
/// login shell: that would run `path_helper`, which moves the system's
/// directories ahead of the stack's on `PATH`.
#[cfg(target_os = "macos")]
fn shell_script(dir: &Path, env: &[(String, String)]) -> String {
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let mut script = String::from("#!/bin/sh\n# Written by SLOPP for its Shell button.\n");
    for (k, v) in env {
        script.push_str(&format!("export {k}={}\n", quote(v)));
    }
    script.push_str(&format!("cd {}\n", quote(&dir.to_string_lossy())));
    script.push_str("exec \"${SHELL:-/bin/zsh}\" -i\n");
    script
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn settings_name_commands_or_applications() {
        let cmd = command_or_app("sh -c");
        assert_eq!(cmd.get_program(), "sh");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["-c"]);

        let cmd = command_or_app("Visual Studio Code");
        assert_eq!(cmd.get_program(), "open");
        assert_eq!(
            cmd.get_args().collect::<Vec<_>>(),
            ["-a", "Visual Studio Code"]
        );
    }

    #[test]
    fn shell_script_quotes_its_values() {
        let env = [("PATH".to_string(), "/a b:/it's".to_string())];
        assert_eq!(
            shell_script(Path::new("/stack"), &env),
            "#!/bin/sh\n# Written by SLOPP for its Shell button.\n\
             export PATH='/a b:/it'\\''s'\ncd '/stack'\nexec \"${SHELL:-/bin/zsh}\" -i\n"
        );
    }
}
