//! Finds the server programs: Apache httpd and its modules, PHP (as an Apache
//! module or PHP-FPM) and MariaDB. Settings can point at any of them; the
//! rest are looked up on `PATH` and in the usual system locations.

use std::path::{Path, PathBuf};

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
    /// On Windows `php8apache2_4.dll` from PHP's Thread Safe build, in PHP's
    /// own folder.
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
    // Where this httpd was built to look, e.g. Homebrew's etc/httpd.
    if let Some(types) = info
        .as_deref()
        .and_then(|v| parse_define(v, "AP_TYPES_CONFIG_FILE"))
    {
        let root = httpd_root.as_deref().unwrap_or("/");
        mime.push(Path::new(root).join(types));
    }
    if let Some(root) = &httpd_root {
        mime.push(PathBuf::from(root).join("conf/mime.types"));
    }
    // Apache for Windows is unpacked anywhere, whatever it was built for.
    if let Some(prefix) = bin.parent().and_then(Path::parent) {
        mime.push(prefix.join("conf/mime.types"));
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
        if cfg!(windows)
            && let Some(dir) = bin.parent()
        {
            libphp.push(dir.join("php8apache2_4.dll"));
            libphp.push(dir.join("php7apache2_4.dll"));
        }
        if let Some(prefix) = bin.parent().and_then(Path::parent) {
            libphp.push(prefix.join("modules/libphp.so"));
            libphp.push(prefix.join("lib/apache2/modules/libphp.so"));
            // Homebrew's php.
            libphp.push(prefix.join("lib/httpd/modules/libphp.so"));
        }
    }
    if httpd.is_some_and(|h| !loads_third_party_modules(&h.bin)) {
        libphp.clear();
    } else if let Some(dir) = httpd.and_then(|h| h.modules.as_ref())
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

/// Whether this httpd takes a `LoadModule` for a module it didn't ship with.
/// The Apache that comes with macOS refuses modules that aren't code-signed
/// (Homebrew's `libphp.so` isn't), so PHP goes through PHP-FPM there.
fn loads_third_party_modules(httpd: &Path) -> bool {
    !(cfg!(target_os = "macos") && httpd.starts_with("/usr/sbin"))
}

fn detect_mariadb(overrides: &Binaries) -> Option<MariaDb> {
    let server = overrides
        .mariadbd
        .clone()
        .or_else(|| find_program(&["mariadbd", "mysqld"]))
        .or_else(|| find_in(Path::new(LIBEXEC_DIR), &["mariadbd", "mysqld"]))?;
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
#[cfg(unix)]
const SYSTEM_DIRS: &[&str] = &["/usr/sbin", "/usr/local/sbin", "/sbin"];

/// Where Fedora and RHEL install `mariadbd`, off every `PATH`. Searched only
/// for the server, after `PATH` and [`SYSTEM_DIRS`].
const LIBEXEC_DIR: &str = "/usr/libexec";

/// Where Homebrew (Apple silicon, Intel) and MacPorts install.
#[cfg(target_os = "macos")]
const PACKAGE_PREFIXES: &[&str] = &["/opt/homebrew", "/usr/local", "/opt/local"];

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    #[cfg(target_os = "macos")]
    {
        // An app started from Finder or the Dock gets a bare system PATH, so
        // add the package managers' directories ourselves.
        for prefix in PACKAGE_PREFIXES {
            dirs.push(Path::new(prefix).join("bin"));
            dirs.push(Path::new(prefix).join("sbin"));
            dirs.extend(keg_dirs(&Path::new(prefix).join("opt")));
        }
        // They go ahead of the Apache that comes with macOS, wherever PATH
        // had it: the same programs are found from a terminal and the Dock.
        let (system, mut dirs): (Vec<_>, Vec<_>) = dirs
            .into_iter()
            .partition(|d| d == Path::new("/usr/sbin") || d == Path::new("/usr/bin"));
        dirs.extend(system);
        dirs.extend(SYSTEM_DIRS.iter().map(PathBuf::from));
        dirs
    }
    #[cfg(windows)]
    {
        dirs.extend(windows_dirs());
        dirs
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        dirs.extend(SYSTEM_DIRS.iter().map(PathBuf::from));
        dirs
    }
}

/// Where the servers go on Windows, which has no common prefix for them:
/// Apache Lounge's and PHP's zips unpacked as their instructions say,
/// Chocolatey's packages, MariaDB's installer and XAMPP's own copies.
#[cfg(windows)]
fn windows_dirs() -> Vec<PathBuf> {
    let mut drive = std::env::var_os("SystemDrive").unwrap_or_else(|| "C:".into());
    drive.push("\\");
    let drive = PathBuf::from(drive);
    let program_files = std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| drive.join("Program Files"));
    let mut dirs = vec![drive.join("Apache24\\bin")];
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(Path::new(&appdata).join("Apache24\\bin"));
    }
    dirs.push(drive.join("php"));
    dirs.extend(versioned_dirs(&drive.join("tools"), "php"));
    dirs.extend(versioned_dirs(&drive, "php"));
    dirs.extend(
        versioned_dirs(&program_files, "MariaDB")
            .into_iter()
            .map(|d| d.join("bin")),
    );
    let xampp = drive.join("xampp");
    dirs.extend(["apache\\bin", "php", "mysql\\bin"].map(|d| xampp.join(d)));
    dirs
}

/// The folders in `parent` whose names start with `prefix` (any case),
/// newest version first: `MariaDB 11.8` before `MariaDB 10.11`.
#[cfg(any(windows, test))]
fn versioned_dirs(parent: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut found: Vec<(Vec<u32>, PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let rest = name
                .get(..prefix.len())?
                .eq_ignore_ascii_case(prefix)
                .then(|| &name[prefix.len()..])?;
            let version = rest
                .split(|c: char| !c.is_ascii_digit())
                .filter_map(|part| part.parse().ok())
                .collect();
            Some((version, e.path()))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, dir)| dir).collect()
}

/// `bin` and `sbin` of the Homebrew formulae we use that aren't linked into
/// the prefix: versioned ones like `mariadb@11.8` are "keg-only", and
/// `mariadb` itself can't be linked next to `mysql`. Unversioned first, then
/// the newest version.
#[cfg(target_os = "macos")]
fn keg_dirs(opt: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(opt) else {
        return Vec::new();
    };
    let mut kegs: Vec<(Vec<u32>, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            Some((keg_version(&name)?, e.path()))
        })
        .collect();
    kegs.sort_by(|a, b| b.0.cmp(&a.0));
    kegs.into_iter()
        .flat_map(|(_, keg)| [keg.join("bin"), keg.join("sbin")])
        .collect()
}

/// A sort key for `httpd`, `php`, `mariadb` and their `@version` variants;
/// `None` for any other formula. The unversioned formula sorts highest.
#[cfg(target_os = "macos")]
fn keg_version(name: &str) -> Option<Vec<u32>> {
    let (formula, version) = match name.split_once('@') {
        Some((formula, version)) => (formula, Some(version)),
        None => (name, None),
    };
    if !["httpd", "php", "mariadb"].contains(&formula) {
        return None;
    }
    match version {
        None => Some(vec![u32::MAX]),
        Some(v) => v.split('.').map(|part| part.parse().ok()).collect(),
    }
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
    names
        .iter()
        .find_map(|name| dirs.iter().find_map(|dir| find_in(dir, &[name])))
}

/// The first of `names` in `dir`, resolved through symlinks (and Scoop's
/// shims, which stand in for programs installed elsewhere).
fn find_in(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .map(|name| dir.join(exe(name)))
        .find(|p| is_executable(p))
        .map(|p| shim_target(&p).unwrap_or_else(|| real_path(&p)))
}

/// `path` with symlinks resolved. On Windows without the `\\?\` prefix,
/// which neither Apache's config nor most programs understand.
fn real_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    let real = dunce::canonicalize(path);
    #[cfg(not(windows))]
    let real = path.canonicalize();
    real.unwrap_or_else(|_| path.to_path_buf())
}

/// The program a Scoop shim (`httpd.exe` beside `httpd.shim`) runs: Apache's
/// modules and PHP's DLLs are found beside it, not beside the shim.
fn shim_target(program: &Path) -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let text = std::fs::read_to_string(program.with_extension("shim")).ok()?;
    parse_shim(&text).filter(|p| p.is_file())
}

/// `path = "C:\...\httpd.exe"` in a `.shim` file.
fn parse_shim(text: &str) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "path").then(|| PathBuf::from(value.trim().trim_matches('"')))
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
    let out = crate::process::command(bin).args(args).output().ok()?;
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
    fn parses_mime_types_define() {
        let info = " -D HTTPD_ROOT=\"/usr\"\n -D AP_TYPES_CONFIG_FILE=\"/private/etc/apache2/mime.types\"\n";
        assert_eq!(
            parse_define(info, "AP_TYPES_CONFIG_FILE").as_deref(),
            Some("/private/etc/apache2/mime.types")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn orders_homebrew_kegs() {
        assert_eq!(keg_version("openssl@3"), None);
        assert_eq!(keg_version("mariadb-connector-c"), None);
        assert_eq!(keg_version("php@8.4"), Some(vec![8, 4]));
        assert!(keg_version("mariadb") > keg_version("mariadb@11.8"));
        assert!(keg_version("mariadb@11.8") > keg_version("mariadb@10.11"));

        let dir = tempfile::tempdir().unwrap();
        for keg in ["mariadb@10.11", "mariadb@11.8", "mysql", "php"] {
            std::fs::create_dir_all(dir.path().join(keg)).unwrap();
        }
        let dirs = keg_dirs(dir.path());
        assert_eq!(dirs.len(), 6);
        assert_eq!(dirs[0], dir.path().join("php/bin"));
        assert_eq!(dirs[2], dir.path().join("mariadb@11.8/bin"));
        assert_eq!(dirs[4], dir.path().join("mariadb@10.11/bin"));
    }

    #[test]
    fn apples_httpd_gets_php_fpm() {
        assert_eq!(
            loads_third_party_modules(Path::new("/usr/sbin/httpd")),
            !cfg!(target_os = "macos")
        );
        assert!(loads_third_party_modules(Path::new(
            "/opt/homebrew/Cellar/httpd/2.4.69/bin/httpd"
        )));
    }

    #[test]
    fn reads_scoop_shims() {
        assert_eq!(
            parse_shim("path = \"C:\\Users\\me\\scoop\\apps\\php\\current\\php.exe\"\nargs = -n\n"),
            Some(PathBuf::from(r"C:\Users\me\scoop\apps\php\current\php.exe"))
        );
        assert_eq!(parse_shim("args = -n\n"), None);
    }

    #[test]
    fn orders_versioned_dirs() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "MariaDB 10.11",
            "MariaDB 11.8",
            "mariadb 11.4",
            "MySQL",
            "php84",
        ] {
            std::fs::create_dir_all(dir.path().join(name)).unwrap();
        }
        std::fs::write(dir.path().join("MariaDB 12.0"), "").unwrap();
        assert_eq!(
            versioned_dirs(dir.path(), "MariaDB"),
            ["MariaDB 11.8", "mariadb 11.4", "MariaDB 10.11"].map(|n| dir.path().join(n))
        );
        assert_eq!(
            versioned_dirs(dir.path(), "php"),
            [dir.path().join("php84")]
        );
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
            parse_mariadb_version(
                "/opt/homebrew/opt/mariadb@11.8/bin/mariadbd  Ver 11.8.9-MariaDB for osx10.23 on arm64 (Homebrew)"
            )
            .as_deref(),
            Some("11.8.9")
        );
        assert_eq!(
            parse_mariadb_version("mysqld  Ver 8.0.36 for Linux on x86_64 (MySQL)").as_deref(),
            Some("8.0.36")
        );
    }

    #[cfg(unix)]
    #[test]
    fn finds_executables_in_a_dir() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let make = |name: &str, mode: u32| {
            let path = dir.path().join(name);
            std::fs::write(&path, "").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        assert_eq!(find_in(dir.path(), &["mariadbd", "mysqld"]), None);

        make("mariadbd", 0o644);
        assert_eq!(find_in(dir.path(), &["mariadbd", "mysqld"]), None);

        // Fedora's /usr/libexec/mysqld is a symlink to mariadbd.
        make("mariadbd", 0o755);
        std::os::unix::fs::symlink("mariadbd", dir.path().join("mysqld")).unwrap();
        let real = dir.path().join("mariadbd").canonicalize().unwrap();
        assert_eq!(find_in(dir.path(), &["mysqld"]), Some(real.clone()));
        assert_eq!(find_in(dir.path(), &["mariadbd", "mysqld"]), Some(real));
    }
}
