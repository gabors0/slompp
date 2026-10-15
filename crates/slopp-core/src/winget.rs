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

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::bail;

use crate::install::{self, Package};

/// Where to get winget when it's missing: App Installer in the Microsoft
/// Store, which brings it.
pub const GET_WINGET_URL: &str = "https://aka.ms/getwinget";

/// The PHP series installed. Its package keeps to that series, so an
/// update never brings a new minor version that a site isn't ready for.
const PHP_SERIES: &str = "8.5";

/// `winget install`'s exit codes when the package is installed already and
/// no newer version applies (`APPINSTALLER_CLI_ERROR_UPDATE_NOT_APPLICABLE`,
/// `APPINSTALLER_CLI_ERROR_PACKAGE_ALREADY_INSTALLED`).
const UPDATE_NOT_APPLICABLE: i32 = 0x8A15002B_u32 as i32;
const PACKAGE_ALREADY_INSTALLED: i32 = 0x8A150061_u32 as i32;

/// The download doesn't match the hash in winget's package
/// (`APPINSTALLER_CLI_ERROR_INSTALLER_HASH_MISMATCH`): the publisher
/// replaced the file and the community repository hasn't caught up yet.
const INSTALLER_HASH_MISMATCH: i32 = 0x8A150011_u32 as i32;

/// The winget package identifier.
pub fn id(package: Package) -> String {
    match package {
        Package::Apache => "ApacheLounge.httpd".into(),
        Package::Php => format!("PHP.PHP.{PHP_SERIES}"),
        Package::MariaDb => "MariaDB.Server".into(),
    }
}

fn publisher(package: Package) -> &'static str {
    match package {
        Package::Apache => "Apache Lounge",
        Package::Php => "php.net",
        Package::MariaDb => "mariadb.org",
    }
}

/// Where to get it by hand, where SLOPP finds it.
fn by_hand(package: Package) -> &'static str {
    match package {
        Package::Apache => "get Apache from apachelounge.com and unpack it to C:\\Apache24",
        Package::Php => "get PHP's Thread Safe zip from windows.php.net and unpack it to C:\\php",
        Package::MariaDb => "get MariaDB's installer from mariadb.org",
    }
}

fn install_args(package: Package) -> Vec<String> {
    // No `--silent`: for an MSI that would be `/quiet`, which can't ask
    // for elevation. winget's default shows the MSI's progress bar and
    // the UAC prompt, and still skips the setup dialogs.
    [
        "install",
        "--id",
        &id(package),
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
    let mut cmd = crate::process::command(winget);
    cmd.args(install_args(package)).stdin(Stdio::null());
    let outcome = install::run(cmd, &mut progress)?;

    match outcome.code {
        Some(0 | UPDATE_NOT_APPLICABLE | PACKAGE_ALREADY_INSTALLED) => Ok(()),
        // `--ignore-security-hash` would install an unchecked download (and
        // needs an admin to allow it), so say where else to get it.
        Some(INSTALLER_HASH_MISMATCH) => bail!(
            "winget's package doesn't match the file {} now serves; winget's \
             repository usually catches up within days. Try again later, or {}.",
            publisher(package),
            by_hand(package)
        ),
        code => {
            let detail = outcome.detail();
            let code = code.map_or("no exit code".into(), |c| {
                format!("exit code {:#X}", c as u32)
            });
            if detail.is_empty() {
                bail!("winget failed ({code})");
            }
            bail!("winget failed ({code}): {detail}")
        }
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
        let args = install_args(Package::Php);
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
}
