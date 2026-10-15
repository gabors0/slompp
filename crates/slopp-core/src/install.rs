//! Installing missing servers with the system's own package manager: winget
//! on Windows, Homebrew on macOS and the distribution's package manager on
//! Linux, run as root through `pkexec`. SLOPP never installs a package
//! manager itself; when there is none it says how to get the servers.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context as _;

use crate::Module;
#[cfg(target_os = "macos")]
use crate::brew;
#[cfg(target_os = "linux")]
use crate::linux;
#[cfg(windows)]
use crate::winget;

/// What a module needs installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Package {
    Apache,
    Php,
    MariaDb,
}

impl Package {
    pub const ALL: [Package; 3] = [Package::Apache, Package::Php, Package::MariaDb];

    pub fn name(self) -> &'static str {
        match self {
            Package::Apache => "Apache",
            Package::Php => "PHP",
            Package::MariaDb => "MariaDB",
        }
    }

    /// The module that runs it.
    pub fn module(self) -> Module {
        match self {
            Package::Apache | Package::Php => Module::Apache,
            Package::MariaDb => Module::MySql,
        }
    }
}

/// "Apache and PHP", "Apache, PHP and MariaDB".
pub fn names(packages: &[Package]) -> String {
    let names: Vec<&str> = packages.iter().map(|p| p.name()).collect();
    match names.split_last() {
        None => String::new(),
        Some((last, [])) => last.to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// What installs the servers here, for a tooltip: "with winget".
pub const VIA: &str = if cfg!(windows) {
    "with winget"
} else if cfg!(target_os = "macos") {
    "with Homebrew"
} else {
    "with the system's package manager"
};

/// The package manager that installs the servers.
#[derive(Debug, Clone)]
pub enum Installer {
    #[cfg(windows)]
    Winget(PathBuf),
    #[cfg(target_os = "macos")]
    Brew(PathBuf),
    #[cfg(target_os = "linux")]
    Linux(linux::Installer),
}

/// Why nothing can install the servers here, and what to do instead.
#[derive(Debug, Clone)]
pub struct Unavailable {
    /// For the log: the whole story, with links or commands.
    pub message: String,
    /// One line for a notification.
    pub notice: String,
}

impl Installer {
    /// This system's package manager, if SLOPP can drive it. `packages` go
    /// into the advice when it can't.
    pub fn find(packages: &[Package]) -> Result<Installer, Unavailable> {
        #[cfg(windows)]
        {
            let _ = packages;
            winget::find()
                .map(Installer::Winget)
                .ok_or_else(|| Unavailable {
                    message: format!(
                        "Installing the servers needs winget, which comes with App Installer \
                     from the Microsoft Store: {}",
                        winget::GET_WINGET_URL
                    ),
                    notice: "winget isn't installed: get App Installer from the Microsoft Store"
                        .into(),
                })
        }
        #[cfg(target_os = "macos")]
        {
            let _ = packages;
            brew::find()
                .map(Installer::Brew)
                .ok_or_else(|| Unavailable {
                    message: format!(
                        "Installing the servers needs Homebrew ({}). Or install Apache, PHP and \
                     MariaDB another way and set their paths in Settings.",
                        brew::HOMEBREW_URL
                    ),
                    notice: "Homebrew isn't installed: see brew.sh".into(),
                })
        }
        #[cfg(target_os = "linux")]
        {
            linux::Installer::find(packages).map(Installer::Linux)
        }
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        {
            let _ = packages;
            Err(Unavailable {
                message: "Install Apache, PHP and MariaDB with your system's package manager."
                    .into(),
                notice: "Install the servers with your package manager".into(),
            })
        }
    }

    /// "winget", "Homebrew", "apt".
    pub fn name(&self) -> &'static str {
        match self {
            #[cfg(windows)]
            Installer::Winget(_) => "winget",
            #[cfg(target_os = "macos")]
            Installer::Brew(_) => "Homebrew",
            #[cfg(target_os = "linux")]
            Installer::Linux(linux) => linux.name(),
        }
    }

    /// Installs `packages`, passing each line the package manager prints to
    /// `progress`. Returns once it's done; one package failing doesn't stop
    /// the others where the package manager installs them one by one.
    pub fn install(
        &self,
        packages: &[Package],
        progress: impl FnMut(String),
    ) -> anyhow::Result<()> {
        match self {
            #[cfg(windows)]
            Installer::Winget(winget) => {
                let mut progress = progress;
                let mut errors = Vec::new();
                for &package in packages {
                    progress(format!("{}: starting winget…", package.name()));
                    let result = winget::install(winget, package, |line| {
                        progress(format!("{}: {line}", package.name()))
                    });
                    if let Err(err) = result {
                        errors.push(format!("Installing {} failed: {err:#}", package.name()));
                    }
                }
                if errors.is_empty() {
                    Ok(())
                } else {
                    anyhow::bail!("{}", errors.join("\n"))
                }
            }
            #[cfg(target_os = "macos")]
            Installer::Brew(brew) => brew::install(brew, packages, progress)
                .with_context(|| format!("Installing {} failed", names(packages))),
            #[cfg(target_os = "linux")]
            Installer::Linux(linux) => linux
                .install(packages, progress)
                .with_context(|| format!("Installing {} failed", names(packages))),
        }
    }
}

/// How a package manager's run ended.
pub(crate) struct Outcome {
    pub code: Option<i32>,
    /// The last line shown, for an error message.
    pub last: Option<String>,
    /// What it printed on stderr.
    pub errors: String,
}

impl Outcome {
    /// The last line shown and stderr, without repeats: what went wrong.
    pub fn detail(&self) -> String {
        let mut detail: Vec<&str> = self
            .last
            .iter()
            .map(String::as_str)
            .chain(self.errors.lines().map(str::trim).filter(|l| !l.is_empty()))
            .collect();
        detail.dedup();
        detail.join(" ")
    }
}

/// Runs `cmd`, passing each line it prints on stdout to `progress` and
/// keeping stderr for an error message.
pub(crate) fn run(mut cmd: Command, progress: &mut dyn FnMut(String)) -> anyhow::Result<Outcome> {
    let program = Path::new(cmd.get_program())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {program}"))?;

    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let errors = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let mut lines = Lines::default();
    let mut buf = [0u8; 4096];
    loop {
        match stdout.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                for line in lines.push(&buf[..n]) {
                    progress(line);
                }
            }
        }
    }
    for line in lines.push(b"\n") {
        progress(line);
    }
    let status = child
        .wait()
        .with_context(|| format!("waiting for {program}"))?;
    Ok(Outcome {
        code: status.code(),
        last: lines.last,
        errors: crate::process::decode(&errors.join().unwrap_or_default()),
    })
}

/// Splits a package manager's output into lines worth showing. Progress
/// bars and spinners redraw with `\r`, so each redraw is a line of its own;
/// lines without a letter (a spinner, a bare bar) are dropped.
#[derive(Default)]
struct Lines {
    pending: Vec<u8>,
    /// The last line shown.
    last: Option<String>,
}

impl Lines {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(end) = self.pending.iter().position(|&b| b == b'\n' || b == b'\r') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let text = crate::process::decode(&line[..line.len() - 1]);
            let text = text.trim();
            if text.chars().any(char::is_alphabetic) {
                self.last = Some(text.to_string());
                out.push(text.to_string());
            }
        }
        out
    }
}

/// The first of `paths` that is an executable file.
pub(crate) fn first_program(paths: &[&str]) -> Option<PathBuf> {
    paths
        .iter()
        .map(PathBuf::from)
        .find(|p| crate::toolchain::is_executable(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_packages_in_a_sentence() {
        assert_eq!(names(&[]), "");
        assert_eq!(names(&[Package::Php]), "PHP");
        assert_eq!(names(&[Package::Apache, Package::Php]), "Apache and PHP");
        assert_eq!(names(&Package::ALL), "Apache, PHP and MariaDB");
    }

    #[test]
    fn splits_progress_into_lines() {
        let mut lines = Lines::default();
        assert!(
            lines
                .push(b"Found PHP 8.5 [PHP.PHP.8.5] Version")
                .is_empty()
        );
        assert_eq!(
            lines.push(b" 8.5.10\r\n"),
            ["Found PHP 8.5 [PHP.PHP.8.5] Version 8.5.10"]
        );
        assert_eq!(
            lines.push("\r   - \r   \\ \r  ██████  1.00 MB / 32.0 MB\r".as_bytes()),
            ["██████  1.00 MB / 32.0 MB"]
        );
        assert_eq!(lines.last.as_deref(), Some("██████  1.00 MB / 32.0 MB"));
    }

    #[test]
    fn details_skip_repeats() {
        let outcome = Outcome {
            code: Some(1),
            last: Some("Error: no such formula".into()),
            errors: "Error: no such formula\n\nsee the docs\n".into(),
        };
        assert_eq!(outcome.detail(), "Error: no such formula see the docs");
    }
}
