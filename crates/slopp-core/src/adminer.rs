//! Adminer, a database manager in one PHP file. SLOPP downloads the MySQL
//! build from adminer.org into `adminer/adminer.php` and serves it through an
//! `index.php` of its own that logs in as root, like phpMyAdmin's config, and
//! styles it with `adminer.css` in the panel's colours.

use anyhow::{Context as _, bail};

use crate::dbadmin::{Installed, Progress, download};
use crate::setup::write_once;
use crate::{Layout, templates};

/// Redirects to the newest MySQL-only release on GitHub.
const LATEST_URL: &str = "https://www.adminer.org/latest-mysql.php";

pub fn installed(layout: &Layout) -> bool {
    layout.adminer_php().is_file() && layout.adminer_index().is_file()
}

/// Downloads the latest Adminer into `adminer/`, writes its `index.php` and
/// `adminer.css` unless they exist, and adds `/adminer` to Apache's config if it isn't there.
/// Blocking; run it off the UI thread.
pub fn install(
    layout: &Layout,
    mysql_port: u16,
    mut progress: impl FnMut(Progress),
) -> anyhow::Result<Installed> {
    progress(Progress::Checking);
    let (version, url) = latest_release()?;

    let staging = layout.tmp().join("adminer-download");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let file = staging.join("adminer.php");
    download(&url, &file, |fraction| {
        progress(Progress::Downloading {
            version: version.clone(),
            fraction,
        })
    })?;
    let text = std::fs::read_to_string(&file).context("reading the downloaded Adminer")?;
    if !text.starts_with("<?php") || !text.contains("Adminer") {
        bail!("the download doesn't look like Adminer");
    }

    std::fs::create_dir_all(layout.adminer())?;
    std::fs::rename(&file, layout.adminer_php())
        .with_context(|| format!("moving Adminer to {}", layout.adminer().display()))?;
    write_once(
        &layout.adminer_index(),
        &templates::adminer_index(layout, mysql_port),
    )?;
    write_once(&layout.adminer_css(), templates::ADMINER_CSS)?;
    let apache_conf_changed = add_alias(layout)?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok(Installed {
        version,
        apache_conf_changed,
    })
}

/// Stacks set up before Adminer was offered have no `/adminer` in
/// `httpd-slopp.conf`: append it. New stacks get it from the template.
fn add_alias(layout: &Layout) -> anyhow::Result<bool> {
    let path = layout.httpd_slopp_conf();
    let Ok(conf) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    if conf.contains("Alias /adminer") {
        return Ok(false);
    }
    let mut conf = conf;
    if !conf.ends_with('\n') {
        conf.push('\n');
    }
    conf.push('\n');
    conf.push_str(&templates::adminer_alias(layout));
    std::fs::write(&path, conf).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// `(version, file url)` of the newest release, read from where
/// adminer.org's "latest" link points.
fn latest_release() -> anyhow::Result<(String, String)> {
    let response = ureq::get(LATEST_URL)
        .config()
        .max_redirects(0)
        .build()
        .call()
        .context("asking adminer.org for the latest version")?;
    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .context("adminer.org didn't point to a release")?;
    let version = version_from_url(location)
        .with_context(|| format!("adminer.org pointed to an unexpected file: {location}"))?;
    Ok((version, location.to_string()))
}

/// `…/v5.4.1/adminer-5.4.1-mysql.php` → `5.4.1`.
fn version_from_url(url: &str) -> Option<String> {
    let file = url.rsplit('/').next()?;
    let version = file.strip_prefix("adminer-")?.strip_suffix("-mysql.php")?;
    let valid = !version.is_empty()
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    valid.then(|| version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_from_the_release_url() {
        assert_eq!(
            version_from_url(
                "https://github.com/vrana/adminer/releases/download/v5.4.1/adminer-5.4.1-mysql.php"
            )
            .as_deref(),
            Some("5.4.1")
        );
        assert_eq!(version_from_url("https://www.adminer.org/"), None);
        assert_eq!(version_from_url("https://x/adminer-5.4.1.php"), None);
    }

    #[test]
    fn adds_the_alias_once() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        let path = layout.httpd_slopp_conf();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# mine").unwrap();

        assert!(add_alias(&layout).unwrap());
        let conf = std::fs::read_to_string(&path).unwrap();
        assert!(conf.starts_with("# mine\n\n"));
        assert!(conf.contains("Alias /adminer"));

        assert!(!add_alias(&layout).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), conf);
    }

    #[test]
    fn new_stacks_have_the_alias_already() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        crate::setup::ensure_layout(&layout, None).unwrap();
        assert!(!add_alias(&layout).unwrap());
    }
}
