//! Finds the server programs: Apache httpd and its modules, PHP (as an Apache
//! module or PHP-FPM) and MariaDB. Settings can point at any of them; the
//! rest are looked up on `PATH` and in the usual system locations.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::settings::Binaries;

#[derive(Debug, Clone)]
pub struct Httpd {
    pub bin: PathBuf,
    pub version: Option<String>,
    /// Directory holding `mod_*.so`.
    pub modules: Option<PathBuf>,
    /// A `mime.types` to copy into the stack's config.
    pub mime_types: Option<PathBuf>,
}

impl Httpd {
    pub fn has_module(&self, file: &str) -> bool {
        self.modules
            .as_ref()
            .is_some_and(|dir| dir.join(file).is_file())
    }
}

/// How Apache runs PHP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhpHandler {
    /// `libphp.so` loaded into Apache (XAMPP's way; needs the prefork MPM).
    Module(PathBuf),
    /// A PHP-FPM pool Apache talks to over a socket.
    Fpm(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Php {
    pub handler: PhpHandler,
    pub version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MariaDb {
    pub server: PathBuf,
    pub install_db: Option<PathBuf>,
    pub client: Option<PathBuf>,
    /// The install prefix (`<basedir>/bin/mariadbd`).
    pub basedir: Option<PathBuf>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Toolchain {
    pub httpd: Option<Httpd>,
    pub php: Option<Php>,
    pub mariadb: Option<MariaDb>,
}

impl Toolchain {
    pub fn detect(overrides: &Binaries) -> Toolchain {
        let httpd = detect_httpd(overrides);
        let php = detect_php(overrides, httpd.as_ref());
        let mariadb = detect_mariadb(overrides);
        Toolchain {
            httpd,
            php,
            mariadb,
        }
    }
}

fn detect_httpd(overrides: &Binaries) -> Option<Httpd> {
    let bin = overrides
        .httpd
        .clone()
        .or_else(|| find_program(&["httpd", "apache2"]))?;
    let info = run_capture(&bin, &["-V"]);
    let version = info.as_deref().and_then(parse_httpd_version);
    let httpd_root = info.as_deref().and_then(|v| parse_define(v, "HTTPD_ROOT"));

    let modules = overrides.apache_modules.clone().or_else(|| {
        let mut candidates = Vec::new();
        if let Some(root) = &httpd_root {
            candidates.push(PathBuf::from(root).join("modules"));
            candidates.push(PathBuf::from(root).join("lib/apache2/modules"));
        }
        if let Some(prefix) = bin.parent().and_then(Path::parent) {
            candidates.push(prefix.join("modules"));
            candidates.push(prefix.join("lib/apache2/modules"));
            candidates.push(prefix.join("lib64/httpd/modules"));
            candidates.push(prefix.join("lib/httpd/modules"));
            candidates.push(prefix.join("libexec/apache2"));
        }
        for dir in [
            "/usr/lib/apache2/modules",
            "/usr/lib64/httpd/modules",
            "/usr/lib/httpd/modules",
            "/usr/libexec/apache2",
            "/usr/local/apache2/modules",
        ] {
            candidates.push(dir.into());
        }
        candidates
            .into_iter()
            .find(|dir| dir.join("mod_authz_core.so").is_file())
    });

    let mut mime = Vec::new();
    if let Some(root) = &httpd_root {
        mime.push(PathBuf::from(root).join("conf/mime.types"));
    }
    mime.push("/etc/mime.types".into());
    mime.push("/etc/apache2/mime.types".into());
    let mime_types = mime.into_iter().find(|p| p.is_file());

    Some(Httpd {
        bin,
        version,
        modules,
        mime_types,
    })
}

fn detect_php(overrides: &Binaries, httpd: Option<&Httpd>) -> Option<Php> {
    if let Some(module) = &overrides.php_module {
        return Some(Php {
            handler: PhpHandler::Module(module.clone()),
            version: None,
        });
    }

    let fpm = overrides
        .php_fpm
        .clone()
        .or_else(|| find_program(&["php-fpm"]).or_else(|| find_program_prefixed("php-fpm")));

    // Prefer libphp.so when one sits next to the PHP we found, like XAMPP.
    let mut libphp = Vec::new();
    for bin in [fpm.clone(), find_program(&["php"])].into_iter().flatten() {
        if let Some(prefix) = bin.parent().and_then(Path::parent) {
            libphp.push(prefix.join("modules/libphp.so"));
            libphp.push(prefix.join("lib/apache2/modules/libphp.so"));
        }
    }
    if let Some(dir) = httpd.and_then(|h| h.modules.as_ref())
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("libphp") && n.ends_with(".so"))
            })
            .collect();
        found.sort();
        libphp.extend(found.into_iter().rev());
    }

    let version_of = |bin: &Path| {
        run_capture(bin, &["-v"])
            .as_deref()
            .and_then(parse_php_version)
    };

    if let Some(module) = libphp.into_iter().find(|p| p.is_file()) {
        let version = find_program(&["php"]).as_deref().and_then(version_of);
        return Some(Php {
            handler: PhpHandler::Module(module),
            version,
        });
    }
    let fpm = fpm?;
    let version = version_of(&fpm);
    Some(Php {
        handler: PhpHandler::Fpm(fpm),
        version,
    })
}

fn detect_mariadb(overrides: &Binaries) -> Option<MariaDb> {
    let server = overrides
        .mariadbd
        .clone()
        .or_else(|| find_program(&["mariadbd", "mysqld"]))?;
    let near = |names: &[&str]| -> Option<PathBuf> {
        let dir = server.parent()?;
        names.iter().map(|n| dir.join(exe(n))).find(|p| p.is_file())
    };
    let install_db = overrides
        .mariadb_install_db
        .clone()
        .or_else(|| near(&["mariadb-install-db", "mysql_install_db"]))
        .or_else(|| find_program(&["mariadb-install-db", "mysql_install_db"]));
    let client = overrides
        .mariadb_client
        .clone()
        .or_else(|| near(&["mariadb", "mysql"]))
        .or_else(|| find_program(&["mariadb", "mysql"]));
    let basedir = server
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf);
    let version = run_capture(&server, &["--version"])
        .as_deref()
        .and_then(parse_mariadb_version);
    Some(MariaDb {
        server,
        install_db,
        client,
        basedir,
        version,
    })
}

/// Directories searched after `PATH`: system binaries that are often missing
/// from a user's `PATH` (Debian keeps `apache2` and `mariadbd` in sbin).
const EXTRA_DIRS: &[&str] = &["/usr/sbin", "/usr/local/sbin", "/sbin"];

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if cfg!(unix) {
        dirs.extend(EXTRA_DIRS.iter().map(PathBuf::from));
    }
    dirs
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// The first of `names` found on `PATH`, resolved through symlinks.
pub fn find_program(names: &[&str]) -> Option<PathBuf> {
    let dirs = search_dirs();
    names.iter().find_map(|name| {
        dirs.iter()
            .map(|dir| dir.join(exe(name)))
            .find(|p| is_executable(p))
            .map(|p| p.canonicalize().unwrap_or(p))
    })
}

/// A program whose name starts with `prefix` (`php-fpm8.2`), newest first.
fn find_program_prefixed(prefix: &str) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = search_dirs()
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flat_map(|entries| entries.flatten().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix))
                && is_executable(p)
        })
        .collect();
    found.sort();
    found.pop()
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Runs a program briefly and returns stdout and stderr together.
fn run_capture(bin: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).output().ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some(text)
}

/// `Server version: Apache/2.4.69 (Unix)` → `2.4.69`.
fn parse_httpd_version(info: &str) -> Option<String> {
    let rest = info
        .lines()
        .find_map(|l| l.strip_prefix("Server version:"))?;
    let after = rest.trim().strip_prefix("Apache/")?;
    Some(after.split_whitespace().next()?.to_string())
}

/// ` -D HTTPD_ROOT="/etc/httpd"` → `/etc/httpd`.
fn parse_define(info: &str, name: &str) -> Option<String> {
    info.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("-D")?.trim();
        let value = rest.strip_prefix(name)?.strip_prefix('=')?;
        Some(value.trim_matches('"').to_string())
    })
}

/// `PHP 8.4.26 (cli) ...` → `8.4.26`.
fn parse_php_version(text: &str) -> Option<String> {
    let first = text.lines().next()?;
    let rest = first.strip_prefix("PHP ")?;
    Some(rest.split_whitespace().next()?.to_string())
}

/// `mariadbd  Ver 11.4.12-MariaDB for Linux ...` → `11.4.12`.
fn parse_mariadb_version(text: &str) -> Option<String> {
    let after = text.split(" Ver ").nth(1)?;
    let word = after.split_whitespace().next()?;
    Some(word.split('-').next()?.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTTPD_V: &str = r#"Server version: Apache/2.4.69 (Unix)
Server built:   Sep 29 2026 12:44:27
Server MPM:     event
 -D APR_HAS_SENDFILE
 -D HTTPD_ROOT="/nix/store/abc-apache-httpd-2.4.69"
 -D SERVER_CONFIG_FILE="conf/httpd.conf"
"#;

    #[test]
    fn parses_httpd_info() {
        assert_eq!(parse_httpd_version(HTTPD_V).as_deref(), Some("2.4.69"));
        assert_eq!(
            parse_define(HTTPD_V, "HTTPD_ROOT").as_deref(),
            Some("/nix/store/abc-apache-httpd-2.4.69")
        );
        assert_eq!(parse_define(HTTPD_V, "NOPE"), None);
    }

    #[test]
    fn parses_versions() {
        assert_eq!(
            parse_php_version("PHP 8.4.26 (cli) (built: Sep 22 2026) (NTS)\nCopyright").as_deref(),
            Some("8.4.26")
        );
        assert_eq!(
            parse_mariadb_version("/usr/bin/mariadbd  Ver 11.4.12-MariaDB for Linux on x86_64")
                .as_deref(),
            Some("11.4.12")
        );
        assert_eq!(
            parse_mariadb_version("mysqld  Ver 8.0.36 for Linux on x86_64 (MySQL)").as_deref(),
            Some("8.0.36")
        );
    }
}
