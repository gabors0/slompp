//! First-run setup of a stack root: directories, config files (written only
//! when missing), the localhost certificate and the MariaDB data directory.

use std::path::Path;

use anyhow::{Context as _, bail};

use crate::toolchain::{Httpd, MariaDb};
use crate::{Layout, templates};

/// Creates the directories and any config file that doesn't exist yet.
pub fn ensure_layout(layout: &Layout, httpd: Option<&Httpd>) -> anyhow::Result<()> {
    for dir in [
        layout.htdocs(),
        layout.tmp(),
        layout.sockets(),
        layout.tmp().join("phpmyadmin"),
        layout.run(),
        layout.apache().join("conf/extra"),
        layout.apache_logs(),
        layout.php().join("logs"),
        layout.mysql(),
        layout.mailpit(),
    ] {
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    // Only this user may reach the database and PHP sockets.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(layout.sockets(), std::fs::Permissions::from_mode(0o700));
    }

    write_once(&layout.httpd_conf(), &templates::httpd_conf(layout))?;
    write_once(
        &layout.httpd_slopp_conf(),
        &templates::httpd_slopp_conf(layout),
    )?;
    write_once(&layout.httpd_ssl_conf(), &templates::httpd_ssl_conf(layout))?;
    write_once(&layout.php_ini(), &templates::php_ini(layout))?;
    ensure_php_mail(layout)?;
    write_once(&layout.my_cnf(), &templates::my_cnf(layout))?;

    if !layout.mime_types().exists() {
        let system = httpd
            .and_then(|h| h.mime_types.as_ref())
            .and_then(|p| std::fs::read_to_string(p).ok());
        let text = system.as_deref().unwrap_or(templates::MIME_TYPES);
        write_once(&layout.mime_types(), text)?;
    }

    let htdocs_empty = std::fs::read_dir(layout.htdocs())
        .map(|mut d| d.next().is_none())
        .unwrap_or(false);
    if htdocs_empty {
        write_once(&layout.htdocs().join("index.php"), templates::dashboard())?;
    }
    Ok(())
}

/// Adds the mail section to a php.ini written before SLOPP had Mailpit (on
/// Windows, before it set `sendmail_from`), so `mail()` goes there too. Only
/// while php.ini doesn't mention `sendmail_path` (Windows: `sendmail_from`)
/// at all: commenting the line out keeps it out. Returns whether it added it.
pub fn ensure_php_mail(layout: &Layout) -> anyhow::Result<bool> {
    let path = layout.php_ini();
    let ini =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    if ini.contains(templates::php_mail_setting(cfg!(windows))) {
        return Ok(false);
    }
    let mut ini = ini.trim_end().to_string();
    ini.push('\n');
    ini.push_str(&templates::php_mail_ini(layout));
    std::fs::write(&path, ini).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// Writes [`Layout::sendmail`], which hands PHP's mail to the Mailpit at
/// `mailpit` (or refuses it while there is none). Rewritten on every start
/// of Apache and Mailpit, so it follows the SMTP port and Mailpit's path.
#[cfg(unix)]
pub fn write_sendmail(
    layout: &Layout,
    mailpit: Option<&Path>,
    smtp_port: u16,
) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let path = layout.sendmail();
    std::fs::write(&path, templates::sendmail_script(mailpit, smtp_port))
        .with_context(|| format!("writing {}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .with_context(|| format!("making {} executable", path.display()))?;
    Ok(())
}

/// Writes `contents` unless the file already exists. Returns whether it wrote.
pub fn write_once(path: &Path, contents: &str) -> anyhow::Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// Makes a self-signed certificate for localhost unless one exists.
pub fn ensure_certificate(layout: &Layout) -> anyhow::Result<()> {
    let (cert_path, key_path) = (layout.ssl_cert(), layout.ssl_key());
    if cert_path.exists() && key_path.exists() {
        return Ok(());
    }
    let names = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(names).context("generating a certificate")?;
    for path in [&cert_path, &key_path] {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
    }
    std::fs::write(&cert_path, cert.pem())
        .with_context(|| format!("writing {}", cert_path.display()))?;
    write_private(&key_path, &key_pair.serialize_pem())?;
    Ok(())
}

fn write_private(path: &Path, contents: &str) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("writing {}", path.display()))?;
        file.write_all(contents.as_bytes())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
    }
}

/// Whether MariaDB's system tables exist.
pub fn mysql_initialised(layout: &Layout) -> bool {
    layout.mysql_data().join("mysql").is_dir()
}

/// Creates the system tables with a password-less `root@localhost`, as XAMPP
/// ships. Output goes to `mysql/install.log`.
pub fn init_mysql(layout: &Layout, mariadb: &MariaDb) -> anyhow::Result<()> {
    if mysql_initialised(layout) {
        return Ok(());
    }
    let Some(install_db) = &mariadb.install_db else {
        bail!("mariadb-install-db was not found, so the database can't be created");
    };
    // mariadb-install-db is a shell script that splits its paths on spaces.
    if cfg!(unix)
        && layout
            .root()
            .to_string_lossy()
            .contains(char::is_whitespace)
    {
        bail!(
            "MariaDB can't create its database under {}: the path contains a space. \
             Choose a stack folder without one in Settings.",
            layout.root().display()
        );
    }
    std::fs::create_dir_all(layout.mysql_data())?;
    let datadir = format!("--datadir={}", layout.mysql_data().display());
    let mut cmd = crate::process::command(install_db);
    if cfg!(windows) {
        // A program of its own there, with fewer options: there's no test
        // database, and root gets no password unless one is given. It leaves
        // a my.ini in the data directory, which goes unused.
        cmd.arg(datadir);
    } else {
        cmd.arg(format!("--defaults-file={}", layout.my_cnf().display()))
            .arg(datadir)
            .arg("--auth-root-authentication-method=normal")
            .arg("--skip-test-db");
        if let Some(basedir) = &mariadb.basedir {
            cmd.arg(format!("--basedir={}", basedir.display()));
        }
    }
    let out = cmd
        .output()
        .with_context(|| format!("running {}", install_db.display()))?;
    let mut log = crate::process::decode(&out.stdout);
    log.push_str(&crate::process::decode(&out.stderr));
    let _ = std::fs::write(layout.mysql_install_log(), &log);
    if !out.status.success() || !mysql_initialised(layout) {
        let tail: Vec<&str> = log.lines().rev().take(6).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        // Leave no half-made data directory behind; the next try starts clean.
        let _ = std::fs::remove_dir_all(layout.mysql_data());
        bail!(
            "creating the database failed (see {}):\n{}",
            layout.mysql_install_log().display(),
            tail.join("\n")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_php_ini_gets_the_mail_section_once() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        std::fs::create_dir_all(layout.php()).unwrap();
        std::fs::write(layout.php_ini(), "[PHP]\nengine = On\n\n").unwrap();
        assert!(ensure_php_mail(&layout).unwrap());
        let ini = std::fs::read_to_string(layout.php_ini()).unwrap();
        assert!(
            ini.starts_with("[PHP]\nengine = On\n\n[mail function]\n"),
            "{ini}"
        );
        assert!(!ensure_php_mail(&layout).unwrap());
        assert_eq!(std::fs::read_to_string(layout.php_ini()).unwrap(), ini);

        // Commented out, it stays out.
        let setting = templates::php_mail_setting(cfg!(windows));
        std::fs::write(layout.php_ini(), format!(";{setting} = x\n")).unwrap();
        assert!(!ensure_php_mail(&layout).unwrap());
    }

    #[test]
    fn writes_config_once_and_keeps_edits() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        ensure_layout(&layout, None).unwrap();

        let conf = std::fs::read_to_string(layout.httpd_conf()).unwrap();
        assert!(conf.contains("Listen ${HTTP_PORT}"));
        assert!(conf.contains(&templates::conf_path(&layout.htdocs())));
        assert!(layout.htdocs().join("index.php").is_file());
        assert!(layout.mime_types().is_file());

        std::fs::write(layout.httpd_conf(), "# mine").unwrap();
        std::fs::remove_file(layout.htdocs().join("index.php")).unwrap();
        std::fs::write(layout.htdocs().join("app.php"), "<?php").unwrap();
        ensure_layout(&layout, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(layout.httpd_conf()).unwrap(),
            "# mine"
        );
        assert!(
            !layout.htdocs().join("index.php").exists(),
            "the dashboard only goes into an empty htdocs"
        );
    }

    #[test]
    fn makes_a_certificate_once() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        ensure_certificate(&layout).unwrap();
        let cert = std::fs::read_to_string(layout.ssl_cert()).unwrap();
        assert!(cert.starts_with("-----BEGIN CERTIFICATE-----"));
        ensure_certificate(&layout).unwrap();
        assert_eq!(std::fs::read_to_string(layout.ssl_cert()).unwrap(), cert);
    }
}
