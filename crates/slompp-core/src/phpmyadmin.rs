//! phpMyAdmin, which XAMPP bundles. SLOMPP downloads the current release from
//! phpmyadmin.net on request and configures it for the stack's MariaDB.

use std::io::{Read as _, Write as _};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, bail};

use crate::{Layout, templates};

const VERSION_URL: &str = "https://www.phpmyadmin.net/home_page/version.txt";

pub fn installed(layout: &Layout) -> bool {
    layout.phpmyadmin().join("index.php").is_file()
}

/// What the installer is doing, for a progress display.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    Checking,
    /// `fraction` is `None` when the server didn't say how big the file is.
    Downloading {
        version: String,
        fraction: Option<f32>,
    },
    Unpacking {
        version: String,
    },
}

/// Downloads and unpacks the latest phpMyAdmin into `phpMyAdmin/` and writes
/// its config. Blocking; run it off the UI thread. Returns the version.
pub fn install(
    layout: &Layout,
    mysql_port: u16,
    mut progress: impl FnMut(Progress),
) -> anyhow::Result<String> {
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
        Some(text) => text,
        None => templates::phpmyadmin_config(layout, &secret()?, mysql_port),
    };
    std::fs::write(layout.phpmyadmin_config(), config)?;
    std::fs::create_dir_all(layout.tmp().join("phpmyadmin"))?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok(version)
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

fn download(url: &str, to: &Path, mut progress: impl FnMut(Option<f32>)) -> anyhow::Result<()> {
    let mut response = ureq::get(url)
        .call()
        .with_context(|| format!("downloading {url}"))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(200 * 1024 * 1024)
        .reader();
    let mut file =
        std::fs::File::create(to).with_context(|| format!("writing {}", to.display()))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    let mut last_reported = 0u64;
    progress(total.map(|_| 0.0));
    loop {
        let n = reader.read(&mut buf).context("downloading phpMyAdmin")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        if done - last_reported >= 256 * 1024 {
            last_reported = done;
            progress(total.map(|t| (done as f32 / t as f32).min(1.0)));
        }
    }
    progress(total.map(|_| 1.0));
    Ok(())
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
    fn secrets_are_32_chars_and_differ() {
        let (a, b) = (secret().unwrap(), secret().unwrap());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
