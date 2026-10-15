//! Where everything lives inside a stack root. Mirrors XAMPP's own layout so
//! people find their way around: `htdocs/`, `apache/conf/httpd.conf`,
//! `php/php.ini`, `mysql/data/`, `phpMyAdmin/`, and `adminer/` and
//! `mailpit/` beside them.

use std::path::{Path, PathBuf};

use crate::Module;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `$SLOPP_ROOT`, else `slopp` in the data directory
    /// (`~/.local/share` unless `$XDG_DATA_HOME` says otherwise; on Windows
    /// the local, not roaming, AppData).
    pub fn default_root() -> PathBuf {
        if let Some(root) = std::env::var_os("SLOPP_ROOT") {
            return root.into();
        }
        base_dir("XDG_DATA_HOME", ".local/share", dirs::data_local_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("slopp")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn htdocs(&self) -> PathBuf {
        self.root.join("htdocs")
    }

    /// Sockets, PHP sessions and uploads.
    pub fn tmp(&self) -> PathBuf {
        self.root.join("tmp")
    }

    /// Pid files and generated runtime configuration.
    pub fn run(&self) -> PathBuf {
        self.root.join("run")
    }

    pub fn pid_file(&self, module: Module) -> PathBuf {
        self.run().join(format!("{}.pids", module.id()))
    }

    // Apache

    pub fn apache(&self) -> PathBuf {
        self.root.join("apache")
    }

    pub fn httpd_conf(&self) -> PathBuf {
        self.apache().join("conf/httpd.conf")
    }

    pub fn httpd_ssl_conf(&self) -> PathBuf {
        self.apache().join("conf/extra/httpd-ssl.conf")
    }

    pub fn httpd_slopp_conf(&self) -> PathBuf {
        self.apache().join("conf/extra/httpd-slopp.conf")
    }

    pub fn mime_types(&self) -> PathBuf {
        self.apache().join("conf/mime.types")
    }

    pub fn ssl_cert(&self) -> PathBuf {
        self.apache().join("conf/ssl.crt/server.crt")
    }

    pub fn ssl_key(&self) -> PathBuf {
        self.apache().join("conf/ssl.key/server.key")
    }

    pub fn apache_logs(&self) -> PathBuf {
        self.apache().join("logs")
    }

    pub fn apache_error_log(&self) -> PathBuf {
        self.apache_logs().join("error.log")
    }

    pub fn apache_access_log(&self) -> PathBuf {
        self.apache_logs().join("access.log")
    }

    /// What httpd itself prints before its error log is open.
    pub fn apache_console_log(&self) -> PathBuf {
        self.apache_logs().join("console.log")
    }

    // PHP

    pub fn php(&self) -> PathBuf {
        self.root.join("php")
    }

    pub fn php_ini(&self) -> PathBuf {
        self.php().join("php.ini")
    }

    pub fn php_error_log(&self) -> PathBuf {
        self.php().join("logs/php_error_log")
    }

    pub fn php_fpm_conf(&self) -> PathBuf {
        self.run().join("php-fpm.conf")
    }

    pub fn php_fpm_socket(&self) -> PathBuf {
        self.sockets().join("php-fpm.sock")
    }

    pub fn php_fpm_log(&self) -> PathBuf {
        self.php().join("logs/php-fpm.log")
    }

    /// The `PHP_INI_SCAN_DIR` SLOPP adds, holding the generated `slopp.ini`:
    /// on Windows where PHP's extensions are, elsewhere the extensions PHP
    /// has but doesn't load by itself.
    pub fn php_scan_dir(&self) -> PathBuf {
        self.run().join("php")
    }

    // MariaDB

    pub fn mysql(&self) -> PathBuf {
        self.root.join("mysql")
    }

    pub fn my_cnf(&self) -> PathBuf {
        self.mysql().join("my.cnf")
    }

    pub fn mysql_data(&self) -> PathBuf {
        self.mysql().join("data")
    }

    pub fn mysql_error_log(&self) -> PathBuf {
        self.mysql_data().join("mysql_error.log")
    }

    pub fn mysql_pid_file(&self) -> PathBuf {
        self.run().join("mysqld.pid")
    }

    /// What mariadbd prints before its error log is open.
    pub fn mysql_console_log(&self) -> PathBuf {
        self.mysql().join("console.log")
    }

    /// Output of the one-off database initialisation.
    pub fn mysql_install_log(&self) -> PathBuf {
        self.mysql().join("install.log")
    }

    pub fn mysql_socket(&self) -> PathBuf {
        self.sockets().join("mysql.sock")
    }

    /// Where Unix sockets go: `tmp/`, unless that path is too long for a
    /// socket (108 bytes on Linux, 104 on macOS), then a short per-user, per-root
    /// directory in `/tmp`. Not `$TMPDIR`: the path ends up in config files,
    /// so every process (panel, CLI, dev shell) must agree on it.
    pub fn sockets(&self) -> PathBuf {
        let tmp = self.tmp();
        // The longest socket name we use is `php-fpm.sock`.
        if tmp.join("php-fpm.sock").as_os_str().len() < 100 {
            return tmp;
        }
        let hash = fnv1a(self.root.as_os_str());
        #[cfg(unix)]
        {
            // SAFETY: getuid has no preconditions.
            let uid = unsafe { libc::getuid() };
            PathBuf::from("/tmp").join(format!("slopp-{uid}-{hash:08x}"))
        }
        #[cfg(not(unix))]
        {
            std::env::temp_dir().join(format!("slopp-{hash:08x}"))
        }
    }

    // Mailpit

    pub fn mailpit(&self) -> PathBuf {
        self.root.join("mailpit")
    }

    /// Mailpit as Linux's Install downloads it; the distributions don't
    /// package it.
    pub fn mailpit_bin(&self) -> PathBuf {
        self.mailpit().join("bin/mailpit")
    }

    /// The mail Mailpit caught, kept across restarts.
    pub fn mailpit_db(&self) -> PathBuf {
        self.mailpit().join("mailpit.db")
    }

    /// Mailpit's log: what it prints.
    pub fn mailpit_log(&self) -> PathBuf {
        self.mailpit().join("mailpit.log")
    }

    // phpMyAdmin

    pub fn phpmyadmin(&self) -> PathBuf {
        self.root.join("phpMyAdmin")
    }

    pub fn phpmyadmin_config(&self) -> PathBuf {
        self.phpmyadmin().join("config.inc.php")
    }

    // Adminer

    pub fn adminer(&self) -> PathBuf {
        self.root.join("adminer")
    }

    /// Adminer itself, replaced by each download.
    pub fn adminer_php(&self) -> PathBuf {
        self.adminer().join("adminer.php")
    }

    /// The page Apache serves, which logs in and includes `adminer.php`.
    pub fn adminer_index(&self) -> PathBuf {
        self.adminer().join("index.php")
    }

    /// The design; Adminer picks up `adminer.css` beside the page.
    pub fn adminer_css(&self) -> PathBuf {
        self.adminer().join("adminer.css")
    }
}

/// A base directory: `$var` when it is an absolute path, else the
/// platform's own. macOS gets the XDG default (`under_home`) too rather than
/// `~/Library/Application Support`: the space in that name breaks
/// `mariadb-install-db`, and one location keeps the documentation the same
/// on every Unix.
pub(crate) fn base_dir(
    var: &str,
    under_home: &str,
    platform: fn() -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(var).map(PathBuf::from)
        && dir.is_absolute()
    {
        return Some(dir);
    }
    if cfg!(target_os = "macos") {
        return dirs::home_dir().map(|home| home.join(under_home));
    }
    platform()
}

/// A hash that stays the same across builds (it ends up in config files).
fn fnv1a(s: &std::ffi::OsStr) -> u32 {
    s.as_encoded_bytes().iter().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(*b)).wrapping_mul(0x0100_0193)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sockets_stay_short() {
        let short = Layout::new("/home/me/.local/share/slopp");
        assert_eq!(short.mysql_socket(), short.tmp().join("mysql.sock"));

        let long = Layout::new(format!("/home/me/{}", "deep/".repeat(30)));
        let socket = long.php_fpm_socket();
        assert!(socket.as_os_str().len() < 100, "{}", socket.display());
        assert_eq!(socket, long.php_fpm_socket(), "stable");
    }
}
