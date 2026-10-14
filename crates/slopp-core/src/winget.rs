//! Installing the servers on Windows with winget, its package manager, from
//! the community repository's packages: Apache Lounge's httpd, PHP's Thread
//! Safe build (the one with the Apache module) and MariaDB's MSI. winget
//! picks the versions and pulls in the Visual C++ runtime they need.
//!
//! Apache and PHP are portable packages unpacked under winget's `Packages`
//! folder, which winget adds to the user's `PATH`; a panel started earlier
//! doesn't see that, so [`program_dirs`] lists their folders for
//! [`Toolchain::detect`](crate::Toolchain::detect). MariaDB's MSI goes to
//! `Program Files\MariaDB <version>`, already searched. Run without its
//! dialogs, the MSI registers no Windows service (its only default for
//! `SERVICENAME` is set by a dialog), so nothing else takes MariaDB's port.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context as _, bail};

use crate::Module;

/// Where to get winget when it's missing: App Installer in the Microsoft
/// Store, which brings it.
pub const GET_WINGET_URL: &str = "https://aka.ms/getwinget";

/// The PHP series installed. Its package keeps to that series, so an
/// update never brings a new minor version that a site isn't ready for.
const PHP_SERIES: &str = "8.5";

/// `winget install`'s exit code when the package is installed and no newer
/// version applies (`APPINSTALLER_CLI_ERROR_UPDATE_NOT_APPLICABLE`).
const UPDATE_NOT_APPLICABLE: i32 = 0x8A15002B_u32 as i32;

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

    /// The winget package identifier.
    pub fn id(self) -> String {
        match self {
            Package::Apache => "ApacheLounge.httpd".into(),
            Package::Php => format!("PHP.PHP.{PHP_SERIES}"),
            Package::MariaDb => "MariaDB.Server".into(),
        }
    }

    fn install_args(self) -> Vec<String> {
        // No `--silent`: for an MSI that would be `/quiet`, which can't ask
        // for elevation. winget's default shows the MSI's progress bar and
        // the UAC prompt, and still skips the setup dialogs.
        [
            "install",
            "--id",
            &self.id(),
            "--exact",
            "--source",
            "winget",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--disable-interactivity",
        ]
        .map(String::from)
        .to_vec()
    }
}

/// winget, if this Windows has it.
pub fn find() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    // An app execution alias: a reparse point that `metadata` can't follow,
    // so look at the link itself. Starting it works.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let alias = Path::new(&local).join(r"Microsoft\WindowsApps\winget.exe");
        if alias.symlink_metadata().is_ok() {
            return Some(alias);
        }
    }
    crate::toolchain::find_program(&["winget"])
}

/// Installs `package` with `winget`, passing each line winget prints to
/// `progress`. Returns once winget is done.
pub fn install(
    winget: &Path,
    package: Package,
    mut progress: impl FnMut(String),
) -> anyhow::Result<()> {
    let mut child = crate::process::command(winget)
        .args(package.install_args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting winget")?;

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
    let status = child.wait().context("waiting for winget")?;
    let errors = crate::process::decode(&errors.join().unwrap_or_default());

    match status.code() {
        Some(0) | Some(UPDATE_NOT_APPLICABLE) => Ok(()),
        code => {
            let mut detail: Vec<&str> = lines
                .last
                .iter()
                .map(String::as_str)
                .chain(errors.lines().map(str::trim).filter(|l| !l.is_empty()))
                .collect();
            detail.dedup();
            let code = code.map_or("no exit code".into(), |c| {
                format!("exit code {:#X}", c as u32)
            });
            if detail.is_empty() {
                bail!("winget failed ({code})");
            }
            bail!("winget failed ({code}): {}", detail.join(" "))
        }
    }
}

/// Splits winget's output into lines worth showing. Progress bars and
/// spinners redraw with `\r`, so each redraw is a line of its own; lines
/// without a letter (a spinner, a bare bar) are dropped.
#[derive(Default)]
struct Lines {
    pending: Vec<u8>,
    /// The last line shown, for an error message.
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

/// Folders holding the programs of the portable packages winget installed,
/// for the user and for the machine: Apache Lounge's `Apache24\bin`, then
/// PHP's folders, newest series first.
pub fn program_dirs() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(Path::new(&local).join(r"Microsoft\WinGet\Packages"));
    }
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        roots.push(Path::new(&program_files).join(r"WinGet\Packages"));
    }
    roots
        .iter()
        .flat_map(|root| program_dirs_in(root))
        .collect()
}

/// [`program_dirs`] in one `Packages` folder, whose subfolders are named
/// `<package id>_<source>`.
fn program_dirs_in(packages: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(packages) else {
        return Vec::new();
    };
    let mut apache = Vec::new();
    let mut php: Vec<(Vec<u32>, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some((id, _source)) = name.split_once('_') else {
            continue;
        };
        if id.eq_ignore_ascii_case("ApacheLounge.httpd") {
            apache.push(entry.path().join(r"Apache24\bin"));
        } else if let Some(series) = strip_prefix_ignore_case(id, "PHP.PHP.")
            // The non-thread-safe builds have no Apache module.
            && let Some(version) = series
                .split('.')
                .map(|part| part.parse().ok())
                .collect::<Option<Vec<u32>>>()
        {
            php.push((version, entry.path()));
        }
    }
    php.sort_by(|a, b| b.0.cmp(&a.0));
    apache
        .into_iter()
        .chain(php.into_iter().map(|(_, dir)| dir))
        .collect()
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.get(..prefix.len())?
        .eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_the_exact_package_from_winget() {
        let args = Package::Php.install_args();
        assert_eq!(args[..3], ["install", "--id", "PHP.PHP.8.5"]);
        assert!(args.contains(&"--exact".to_string()));
        assert!(args.windows(2).any(|w| w == ["--source", "winget"]));
        assert!(!args.contains(&"--silent".to_string()));
    }

    #[test]
    fn lists_portable_package_dirs() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "ApacheLounge.httpd_Microsoft.Winget.Source_8wekyb3d8bbwe",
            "PHP.PHP.8.4_Microsoft.Winget.Source_8wekyb3d8bbwe",
            "PHP.PHP.8.5_Microsoft.Winget.Source_8wekyb3d8bbwe",
            "PHP.PHP.NTS.8.5_Microsoft.Winget.Source_8wekyb3d8bbwe",
            "Git.Git_Microsoft.Winget.Source_8wekyb3d8bbwe",
        ] {
            std::fs::create_dir_all(dir.path().join(name)).unwrap();
        }
        let source = "_Microsoft.Winget.Source_8wekyb3d8bbwe";
        assert_eq!(
            program_dirs_in(dir.path()),
            [
                dir.path()
                    .join(format!("ApacheLounge.httpd{source}"))
                    .join(r"Apache24\bin"),
                dir.path().join(format!("PHP.PHP.8.5{source}")),
                dir.path().join(format!("PHP.PHP.8.4{source}")),
            ]
        );
        assert!(program_dirs_in(&dir.path().join("missing")).is_empty());
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
}
