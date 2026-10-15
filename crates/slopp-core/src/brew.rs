//! Installing the servers on macOS with Homebrew, when it's there: `httpd`,
//! `php` (which brings `libphp.so`, so Homebrew's Apache runs PHP as a
//! module), `mariadb` and `mailpit`. Homebrew runs as the user, needs no password and
//! starts no services, so nothing else takes the stack's ports. SLOPP
//! doesn't install Homebrew itself.
//!
//! Homebrew's prefixes are already in [`Toolchain::detect`]'s search
//! (`crate::toolchain`), so the new programs turn up once the stack looks
//! again.
//!
//! [`Toolchain::detect`]: crate::Toolchain::detect

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::bail;

use crate::install::{self, Package};

pub const HOMEBREW_URL: &str = "https://brew.sh";

/// The MariaDB that installs beside Homebrew's `mysql` (or Percona's): the
/// plain `mariadb` formula conflicts with them, this one is keg-only.
const KEG_ONLY_MARIADB: &str = "mariadb@11.8";

/// `brew`, if Homebrew is installed. Looked for in its prefixes first: an app
/// started from the Dock gets a bare `PATH`.
pub fn find() -> Option<PathBuf> {
    let native = if cfg!(target_arch = "aarch64") {
        ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
    } else {
        ["/usr/local/bin/brew", "/opt/homebrew/bin/brew"]
    };
    install::first_program(&native).or_else(|| crate::toolchain::find_program(&["brew"]))
}

/// The formulae for `packages`, given Homebrew's prefix.
fn formulae(prefix: &Path, packages: &[Package]) -> Vec<&'static str> {
    packages
        .iter()
        .map(|package| match package {
            Package::Apache => "httpd",
            Package::Php => "php",
            Package::Mailpit => "mailpit",
            Package::MariaDb => {
                let conflicts = ["mysql", "percona-server"]
                    .iter()
                    .any(|f| prefix.join("opt").join(f).exists());
                if conflicts {
                    KEG_ONLY_MARIADB
                } else {
                    "mariadb"
                }
            }
        })
        .collect()
}

/// Installs `packages` with one `brew install`, passing each line Homebrew
/// prints to `progress`.
pub fn install(
    brew: &Path,
    packages: &[Package],
    mut progress: impl FnMut(String),
) -> anyhow::Result<()> {
    let prefix = brew
        .parent()
        .and_then(Path::parent)
        .unwrap_or(Path::new("/"));
    let mut cmd = crate::process::command(brew);
    cmd.arg("install")
        .args(formulae(prefix, packages))
        .env("HOMEBREW_NO_ENV_HINTS", "1")
        .stdin(Stdio::null());
    let outcome = install::run(cmd, &mut progress)?;
    match outcome.code {
        Some(0) => Ok(()),
        code => {
            let detail = outcome.detail();
            let code = code.map_or("no exit code".into(), |c| format!("exit code {c}"));
            if detail.is_empty() {
                bail!("brew failed ({code})");
            }
            bail!("brew failed ({code}): {detail}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_a_mariadb_that_installs_beside_mysql() {
        let prefix = tempfile::tempdir().unwrap();
        assert_eq!(
            formulae(prefix.path(), &Package::ALL),
            ["httpd", "php", "mariadb", "mailpit"]
        );
        std::fs::create_dir_all(prefix.path().join("opt/mysql")).unwrap();
        assert_eq!(
            formulae(prefix.path(), &[Package::MariaDb]),
            [KEG_ONLY_MARIADB]
        );
    }
}
