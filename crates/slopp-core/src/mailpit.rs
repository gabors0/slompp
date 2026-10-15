//! Getting Mailpit on Linux, where the distributions don't package it: the
//! latest release from GitHub, unpacked into the stack
//! ([`Layout::mailpit_bin`](crate::Layout::mailpit_bin)) and checked against
//! the SHA-256 digest GitHub's API lists for the download. Mailpit is one
//! statically linked program, so the same file runs on every distribution,
//! NixOS included, and installing it needs no password.

use std::io::Read as _;
use std::path::Path;

use anyhow::{Context as _, bail};

const LATEST_URL: &str = "https://api.github.com/repos/axllent/mailpit/releases/latest";

/// A release's download for this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Release {
    /// Without the tag's `v`.
    version: String,
    url: String,
    sha256: String,
}

/// The release's file for a CPU (`std::env::consts::ARCH`), as Mailpit
/// names them.
fn asset_name(arch: &str) -> Option<String> {
    let arch = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "arm" => "arm",
        "x86" => "386",
        _ => return None,
    };
    Some(format!("mailpit-linux-{arch}.tar.gz"))
}

/// The version, URL and digest of `asset` in the API's answer about a
/// release.
fn parse_release(json: &str, asset: &str) -> anyhow::Result<Release> {
    let release: serde_json::Value =
        serde_json::from_str(json).context("GitHub's answer isn't JSON")?;
    let tag = release["tag_name"]
        .as_str()
        .context("GitHub's answer names no release")?;
    let file = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"].as_str() == Some(asset))
        .with_context(|| format!("Mailpit {tag} has no {asset}"))?;
    let url = file["browser_download_url"]
        .as_str()
        .with_context(|| format!("GitHub lists no download for {asset}"))?;
    // Only a digest from GitHub's API makes the download trustworthy, so
    // without one nothing is installed.
    let sha256 = file["digest"]
        .as_str()
        .and_then(|d| d.strip_prefix("sha256:"))
        .with_context(|| format!("GitHub lists no SHA-256 digest for {asset}"))?;
    Ok(Release {
        version: tag.strip_prefix('v').unwrap_or(tag).to_string(),
        url: url.to_string(),
        sha256: sha256.to_ascii_lowercase(),
    })
}

/// Downloads the latest Mailpit to `dest`, replacing what's there, passing
/// what it's doing to `progress`. Returns the version installed.
pub fn install(dest: &Path, mut progress: impl FnMut(String)) -> anyhow::Result<String> {
    let asset = asset_name(std::env::consts::ARCH)
        .with_context(|| format!("Mailpit has no build for {}", std::env::consts::ARCH))?;
    progress("Looking up the latest Mailpit…".into());
    let json = ureq::get(LATEST_URL)
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("asking GitHub for Mailpit's latest release")?
        .body_mut()
        .read_to_string()
        .context("reading GitHub's answer")?;
    let release = parse_release(&json, &asset)?;
    let version = release.version.clone();

    let dir = dest.parent().context("no folder for Mailpit")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let archive = dir.join(&asset);
    let result = (|| {
        let mut shown = None;
        crate::dbadmin::download(&release.url, &archive, |fraction| {
            // Every tenth, not every chunk: each line is shown.
            let tenth = fraction.map(|f| (f * 10.0) as u32);
            if shown != Some(tenth) {
                shown = Some(tenth);
                progress(match tenth {
                    Some(t) => format!("Downloading Mailpit {version}… {}%", t * 10),
                    None => format!("Downloading Mailpit {version}…"),
                });
            }
        })?;
        crate::dbadmin::check_sha256(&archive, &release.sha256)?;
        progress(format!("Unpacking Mailpit {version}…"));
        unpack(&archive, dest)
    })();
    let _ = std::fs::remove_file(&archive);
    result?;
    Ok(version)
}

/// Takes the `mailpit` program out of the release's archive and puts it at
/// `dest`, executable. A new file moved over the old one, so a running
/// Mailpit keeps its own until it restarts.
fn unpack(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let file =
        std::fs::File::open(archive).with_context(|| format!("reading {}", archive.display()))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let entries = tar.entries().context("reading Mailpit's archive")?;
    for entry in entries {
        let mut entry = entry.context("reading Mailpit's archive")?;
        if entry.path().ok().as_deref() != Some(Path::new("mailpit")) {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .context("reading Mailpit's archive")?;
        let new = dest.with_extension("new");
        std::fs::write(&new, bytes).with_context(|| format!("writing {}", new.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))
                .with_context(|| format!("making {} executable", new.display()))?;
        }
        std::fs::rename(&new, dest).with_context(|| format!("writing {}", dest.display()))?;
        return Ok(());
    }
    bail!("Mailpit's archive holds no mailpit program")
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE: &str = r#"{
        "tag_name": "v1.31.6",
        "assets": [
            {
                "name": "mailpit-linux-arm64.tar.gz",
                "digest": "sha256:DE07955B",
                "browser_download_url": "https://github.com/axllent/mailpit/releases/download/v1.31.6/mailpit-linux-arm64.tar.gz"
            },
            {
                "name": "mailpit-linux-386.tar.gz",
                "digest": null,
                "browser_download_url": "https://github.com/axllent/mailpit/releases/download/v1.31.6/mailpit-linux-386.tar.gz"
            }
        ]
    }"#;

    #[test]
    fn names_the_download_for_this_cpu() {
        assert_eq!(
            asset_name("x86_64").as_deref(),
            Some("mailpit-linux-amd64.tar.gz")
        );
        assert_eq!(
            asset_name("aarch64").as_deref(),
            Some("mailpit-linux-arm64.tar.gz")
        );
        assert_eq!(asset_name("riscv64"), None);
    }

    #[test]
    fn reads_the_release_and_insists_on_a_digest() {
        assert_eq!(
            parse_release(RELEASE, "mailpit-linux-arm64.tar.gz").unwrap(),
            Release {
                version: "1.31.6".into(),
                url: "https://github.com/axllent/mailpit/releases/download/v1.31.6/mailpit-linux-arm64.tar.gz".into(),
                sha256: "de07955b".into(),
            }
        );
        let err = parse_release(RELEASE, "mailpit-linux-386.tar.gz").unwrap_err();
        assert!(err.to_string().contains("no SHA-256"), "{err}");
        let err = parse_release(RELEASE, "mailpit-linux-amd64.tar.gz").unwrap_err();
        assert!(err.to_string().contains("has no"), "{err}");
    }

    #[test]
    fn unpacks_only_the_program() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("mailpit.tar.gz");
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(&archive).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gz);
        for (name, body) in [("LICENSE", "MIT"), ("mailpit", "#!/bin/sh\n")] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, body.as_bytes()).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();

        let dest = dir.path().join("bin-mailpit");
        std::fs::write(&dest, "old").unwrap();
        unpack(&archive, &dest).unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "#!/bin/sh\n");
        assert!(!dest.with_extension("new").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = dest.metadata().unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }

        let file = dir.path().join("file");
        std::fs::write(&file, "abc").unwrap();
        crate::dbadmin::check_sha256(
            &file,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .unwrap();
        assert!(crate::dbadmin::check_sha256(&file, "00").is_err());
    }
}
