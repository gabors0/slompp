//! phpMyAdmin, which XAMPP bundles. SLOPP downloads the current release from
//! phpmyadmin.net on request and configures it for the stack's MariaDB,
//! with the dark BooDark theme, in orange, as its default.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, bail};
use sha2::{Digest as _, Sha256};

use crate::dbadmin::{Installed, Progress, download};
use crate::{Layout, templates};

const VERSION_URL: &str = "https://www.phpmyadmin.net/home_page/version.txt";

/// BooDark, a dark Bootstrap theme for phpMyAdmin 5.2, in the orange that
/// matches the panel's accent. (BooDark 1.2.0's own orange scheme is the cyan
/// one relabelled.) Pinned and checked: themes have no "latest" to ask for.
/// Update all three together.
pub const THEME: &str = "boodark-orange";
const THEME_URL: &str =
    "https://files.phpmyadmin.net/themes/boodark-orange/1.1.0/boodark-orange-1.1.0.zip";
const THEME_SHA256: &str = "11311ee95925bce089808bc3905db83412b1951ee99243f5b07986547788082e";

pub fn installed(layout: &Layout) -> bool {
    layout.phpmyadmin().join("index.php").is_file()
}

/// Downloads and unpacks the latest phpMyAdmin and BooDark into `phpMyAdmin/`
/// and writes its config. Blocking; run it off the UI thread.
pub fn install(
    layout: &Layout,
    mysql_port: u16,
    mut progress: impl FnMut(Progress),
) -> anyhow::Result<Installed> {
    progress(Progress::Checking);
    let (version, url) = latest_release()?;

    let staging = layout.tmp().join("phpmyadmin-download");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    let zip_path = staging.join("phpMyAdmin.zip");
    download(&url, &zip_path, |fraction| {
        progress(Progress::Downloading {
            version: version.clone(),
            fraction,
        })
    })?;

    progress(Progress::Unpacking {
        version: version.clone(),
    });
    let unpacked = staging.join("unpacked");
    unpack_stripping_top_dir(&zip_path, &unpacked)?;
    if !unpacked.join("index.php").is_file() {
        bail!("the download doesn't look like phpMyAdmin (no index.php)");
    }
    let theme_zip = staging.join("theme.zip");
    download(THEME_URL, &theme_zip, |_| {})?;
    check_sha256(&theme_zip, THEME_SHA256)?;
    install_theme(&theme_zip, &unpacked.join("themes").join(THEME))?;

    // Keep an existing config (people edit it); replace everything else.
    let target = layout.phpmyadmin();
    let old_config = std::fs::read_to_string(layout.phpmyadmin_config()).ok();
    if target.exists() {
        std::fs::remove_dir_all(&target)
            .with_context(|| format!("removing the old {}", target.display()))?;
    }
    std::fs::rename(&unpacked, &target)
        .with_context(|| format!("moving phpMyAdmin to {}", target.display()))?;
    let config = match old_config {
        Some(text) => with_theme_default(&text).unwrap_or(text),
        None => templates::phpmyadmin_config(layout, &secret()?, mysql_port),
    };
    std::fs::write(layout.phpmyadmin_config(), config)?;
    std::fs::create_dir_all(layout.tmp().join("phpmyadmin"))?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok(Installed {
        version,
        apache_conf_changed: false,
    })
}

fn check_sha256(path: &Path, expected: &str) -> anyhow::Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != expected {
        bail!(
            "{} doesn't match its checksum (expected {expected}, got {actual})",
            path.display()
        );
    }
    Ok(())
}

fn install_theme(zip_path: &Path, dir: &Path) -> anyhow::Result<()> {
    unpack_stripping_top_dir(zip_path, dir)?;
    if !dir.join("theme.json").is_file() {
        bail!("the theme download doesn't look like a phpMyAdmin theme");
    }
    Ok(())
}

/// Makes the theme the default in a config written before SLOPP
/// installed it. `None` when the config already names a default.
fn with_theme_default(config: &str) -> Option<String> {
    if config.contains("ThemeDefault") {
        return None;
    }
    let line = format!("$cfg['ThemeDefault'] = '{THEME}';\n");
    let config = config.trim_end();
    Some(match config.strip_suffix("?>") {
        Some(body) => format!("{}\n{line}?>\n", body.trim_end()),
        None => format!("{config}\n\n{line}"),
    })
}

/// `(version, zip url)` of the newest release.
fn latest_release() -> anyhow::Result<(String, String)> {
    let text = ureq::get(VERSION_URL)
        .call()
        .context("asking phpmyadmin.net for the latest version")?
        .body_mut()
        .read_to_string()
        .context("reading phpmyadmin.net's answer")?;
    parse_version_txt(&text).context("phpmyadmin.net answered in an unexpected format")
}

/// `version.txt` is the version, the release date and a download URL.
fn parse_version_txt(text: &str) -> Option<(String, String)> {
    let mut lines = text.lines().map(str::trim);
    let version = lines.next().filter(|v| !v.is_empty())?.to_string();
    let url = format!(
        "https://files.phpmyadmin.net/phpMyAdmin/{version}/phpMyAdmin-{version}-all-languages.zip"
    );
    Some((version, url))
}

/// Unpacks a zip whose entries share one top directory (`phpMyAdmin-x.y.z/`)
/// into `to`, without that directory.
fn unpack_stripping_top_dir(zip_path: &Path, to: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).context("opening the phpMyAdmin archive")?;
    std::fs::create_dir_all(to)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        // enclosed_name rejects absolute paths and `..`.
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let inner: PathBuf = name
            .components()
            .skip(1)
            .filter(|c| matches!(c, Component::Normal(_)))
            .collect();
        if inner.as_os_str().is_empty() {
            continue;
        }
        let out = to.join(inner);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut dest =
            std::fs::File::create(&out).with_context(|| format!("writing {}", out.display()))?;
        std::io::copy(&mut entry, &mut dest)?;
    }
    Ok(())
}

/// 32 random characters for phpMyAdmin's cookie encryption.
fn secret() -> anyhow::Result<String> {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no randomness available: {e}"))?;
    Ok(bytes
        .iter()
        .map(|b| CHARS[*b as usize % CHARS.len()] as char)
        .collect())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    #[test]
    fn reads_version_txt() {
        let (version, url) =
            parse_version_txt("5.2.2\n2025-01-21\nhttps://files.phpmyadmin.net/x.zip\n").unwrap();
        assert_eq!(version, "5.2.2");
        assert_eq!(
            url,
            "https://files.phpmyadmin.net/phpMyAdmin/5.2.2/phpMyAdmin-5.2.2-all-languages.zip"
        );
        assert!(parse_version_txt("").is_none());
    }

    #[test]
    fn unpacks_without_the_top_directory() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("a.zip");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            zip.add_directory("phpMyAdmin-1.0/", opts).unwrap();
            zip.start_file("phpMyAdmin-1.0/index.php", opts).unwrap();
            zip.write_all(b"<?php").unwrap();
            zip.start_file("phpMyAdmin-1.0/js/app.js", opts).unwrap();
            zip.write_all(b"x").unwrap();
            zip.finish().unwrap();
        }
        let out = dir.path().join("out");
        unpack_stripping_top_dir(&zip_path, &out).unwrap();
        assert!(out.join("index.php").is_file());
        assert!(out.join("js/app.js").is_file());
    }

    #[test]
    fn adds_the_theme_to_old_configs_once() {
        let new = with_theme_default("<?php\n$cfg['x'] = 1;\n").unwrap();
        assert_eq!(
            new,
            "<?php\n$cfg['x'] = 1;\n\n$cfg['ThemeDefault'] = 'boodark-orange';\n"
        );
        assert_eq!(with_theme_default(&new), None);

        let closed = with_theme_default("<?php\n$cfg['x'] = 1;\n?>\n").unwrap();
        assert_eq!(
            closed,
            "<?php\n$cfg['x'] = 1;\n$cfg['ThemeDefault'] = 'boodark-orange';\n?>\n"
        );

        let template = templates::phpmyadmin_config(&Layout::new("/s"), "x", 3306);
        assert_eq!(with_theme_default(&template), None);
    }

    #[test]
    fn checks_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, "abc").unwrap();
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        check_sha256(&path, abc).unwrap();
        assert!(check_sha256(&path, &abc.replace('b', "c")).is_err());
    }

    #[test]
    fn secrets_are_32_chars_and_differ() {
        let (a, b) = (secret().unwrap(), secret().unwrap());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
