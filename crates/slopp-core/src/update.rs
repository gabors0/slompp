//! Updating SLOPP itself from its GitHub releases: whether there is a newer
//! one, how this copy was installed, and replacing it with the new release's
//! download for that kind of install, checked against the SHA-256 digest
//! GitHub's API lists for it (a download without one is refused, as in
//! `site/install.sh`).
//!
//! - The AppImage (`$APPIMAGE`) is downloaded beside itself and moved over
//!   the old one, as `install.sh` does; the running one keeps its file.
//! - `SLOPP.app` is copied out of the disk image beside the old app and
//!   swapped in.
//! - The Windows installer is downloaded and runs when SLOPP restarts,
//!   silently, and starts the new SLOPP itself (`/RELAUNCH=1`, see
//!   `slopp.iss`).
//! - A folder from the tarball or the zip gets the new `slopp` and
//!   `sloppctl`. Windows can't replace a running program, but it can rename
//!   one, so the old ones become `*.old` until the next start removes them.
//!
//! Copies that something else manages (Nix, a build from source, a folder
//! SLOPP can't write to) are only told about new releases.

use std::ffi::OsString;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, bail};

use crate::dbadmin::{self, Progress};

const LATEST_URL: &str = "https://api.github.com/repos/gabors0/slopp/releases/latest";

/// The releases page, for copies SLOPP can't update itself.
pub const RELEASES_PAGE: &str = "https://github.com/gabors0/slopp/releases/latest";

/// The version running.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// A release on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Without the tag's `v`.
    pub version: String,
    /// Its page on GitHub, with the release notes.
    pub page: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Asset {
    name: String,
    url: String,
    /// Lowercase hex, when GitHub lists one.
    sha256: Option<String>,
}

impl Release {
    /// The download whose name ends in `suffix`, as the website matches
    /// them, and its digest.
    fn asset(&self, suffix: &str) -> anyhow::Result<(&str, &str)> {
        let asset = self
            .assets
            .iter()
            .find(|a| a.name.ends_with(suffix))
            .with_context(|| {
                format!("SLOPP {} has no download ending in {suffix}", self.version)
            })?;
        let sha256 = asset
            .sha256
            .as_deref()
            .with_context(|| format!("GitHub lists no SHA-256 digest for {}", asset.name))?;
        Ok((&asset.url, sha256))
    }
}

/// Asks GitHub for the latest release (prereleases aren't). Blocking.
pub fn latest() -> anyhow::Result<Release> {
    let json = ureq::get(LATEST_URL)
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("asking GitHub for SLOPP's latest release")?
        .body_mut()
        .read_to_string()
        .context("reading GitHub's answer")?;
    parse_release(&json)
}

fn parse_release(json: &str) -> anyhow::Result<Release> {
    let release: serde_json::Value =
        serde_json::from_str(json).context("GitHub's answer isn't JSON")?;
    let tag = release["tag_name"]
        .as_str()
        .context("GitHub's answer names no release")?;
    let assets = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some(Asset {
                name: a["name"].as_str()?.to_string(),
                url: a["browser_download_url"].as_str()?.to_string(),
                sha256: a["digest"]
                    .as_str()
                    .and_then(|d| d.strip_prefix("sha256:"))
                    .map(str::to_ascii_lowercase),
            })
        })
        .collect();
    Ok(Release {
        version: tag.strip_prefix('v').unwrap_or(tag).to_string(),
        page: release["html_url"]
            .as_str()
            .unwrap_or(RELEASES_PAGE)
            .to_string(),
        assets,
    })
}

/// Whether version `candidate` comes after `current`, by semantic
/// versioning's precedence (`0.4.0-rc.1` < `0.4.0`). Unreadable versions
/// never do.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (Version::parse(candidate), Version::parse(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// A version's parts in the order they compare.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    numbers: [u64; 3],
    /// A release comes after its prereleases.
    release: bool,
    pre: Vec<Identifier>,
}

/// Numeric identifiers come before alphanumeric ones.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Identifier {
    Number(u64),
    Text(String),
}

impl Version {
    fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        // Build metadata doesn't count.
        let text = text.split('+').next()?;
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut numbers = [0; 3];
        let mut parts = core.split('.');
        for n in &mut numbers {
            *n = parts.next()?.parse().ok()?;
        }
        if parts.next().is_some() {
            return None;
        }
        let release = pre.is_none();
        let pre = pre
            .map(|pre| {
                pre.split('.')
                    .map(|id| match id.parse() {
                        Ok(n) => Identifier::Number(n),
                        Err(_) => Identifier::Text(id.to_string()),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Version {
            numbers,
            release,
            pre,
        })
    }
}

/// How this copy of SLOPP was installed, which says how to update it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Install {
    /// The AppImage, at `$APPIMAGE`.
    AppImage(PathBuf),
    /// The app bundle from the disk image.
    MacApp(PathBuf),
    /// The folder the Windows installer put SLOPP in.
    Setup(PathBuf),
    /// A folder with `slopp` and `sloppctl` from the tarball or the zip.
    Folder(PathBuf),
    /// Something SLOPP doesn't update itself.
    Elsewhere(Elsewhere),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Elsewhere {
    /// From the flake: `nix profile upgrade` or the system's configuration.
    Nix,
    /// A build from a checkout.
    Source,
    /// macOS runs it from a read-only copy (App Translocation), since it was
    /// opened where it was downloaded.
    Translocated,
    /// SLOPP can't write to this folder.
    ReadOnly(PathBuf),
    /// No release is built for this system or CPU.
    NoBuild,
    /// Not a layout any release has.
    Unknown,
}

impl Install {
    /// This copy's install.
    pub fn detect() -> Install {
        let Ok(exe) = std::env::current_exe() else {
            return Install::Elsewhere(Elsewhere::Unknown);
        };
        // The real file, not a symlink to it (a Nix profile's, `slopp` beside
        // the AppImage).
        let exe = exe.canonicalize().unwrap_or(exe);
        let appimage = std::env::var_os("APPIMAGE")
            .filter(|_| cfg!(target_os = "linux"))
            .map(PathBuf::from);
        detect_from(&Probe {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            debug: cfg!(debug_assertions),
            exe: &exe,
            appimage: appimage.as_deref(),
            exists: &|p| p.exists(),
            writable: &writable,
        })
    }

    pub fn can_update(&self) -> bool {
        !matches!(self, Install::Elsewhere(_))
    }

    /// What this copy is, for Settings.
    pub fn describe(&self) -> String {
        match self {
            Install::AppImage(path) => format!("The AppImage at {}", path.display()),
            Install::MacApp(path) => format!("The app at {}", path.display()),
            Install::Setup(dir) => format!("Installed in {}", dir.display()),
            Install::Folder(dir) => format!("The programs in {}", dir.display()),
            Install::Elsewhere(why) => match why {
                Elsewhere::Nix => "Installed with Nix: update it with `nix profile upgrade`, \
                                   or with your system's configuration"
                    .into(),
                Elsewhere::Source => "Built from source: pull and build again to update".into(),
                Elsewhere::Translocated => "macOS runs SLOPP from a temporary copy: move \
                                            SLOPP.app to Applications to update it from here"
                    .into(),
                Elsewhere::ReadOnly(dir) => format!(
                    "SLOPP can't write to {}, so get new versions from the releases page",
                    dir.display()
                ),
                Elsewhere::NoBuild => "There's no release built for this system: build it \
                                       from source"
                    .into(),
                Elsewhere::Unknown => "SLOPP doesn't know how it was installed here, so get \
                                       new versions from the releases page"
                    .into(),
            },
        }
    }

    /// The release download for this install, by the end of its name.
    fn suffix(&self) -> Option<&'static str> {
        Some(match self {
            Install::AppImage(_) => "linux-x86_64.AppImage",
            Install::MacApp(_) => "macos-arm64.dmg",
            Install::Setup(_) => "windows-x86_64-setup.exe",
            Install::Folder(_) if cfg!(windows) => "windows-x86_64.zip",
            Install::Folder(_) => "linux-x86_64.tar.gz",
            Install::Elsewhere(_) => return None,
        })
    }

    /// Downloads `release` and puts it in place of this copy (or, for the
    /// Windows installer, gets it ready to run). Blocking. Returns what
    /// starts the new version once this one quits.
    pub fn install(
        &self,
        release: &Release,
        mut progress: impl FnMut(Progress),
    ) -> anyhow::Result<Restart> {
        let Some(suffix) = self.suffix() else {
            bail!("{}", self.describe());
        };
        let (url, sha256) = release.asset(suffix)?;
        let version = release.version.clone();
        let get = |to: &Path, progress: &mut dyn FnMut(Progress)| {
            fetch(url, sha256, &version, to, progress)
        };
        match self {
            Install::AppImage(path) => {
                let part = sibling(path, "update");
                let result = get(&part, &mut progress).and_then(|()| {
                    progress(Progress::Unpacking {
                        version: version.clone(),
                    });
                    make_executable(&part)?;
                    std::fs::rename(&part, path)
                        .with_context(|| format!("replacing {}", path.display()))
                });
                if result.is_err() {
                    let _ = std::fs::remove_file(&part);
                }
                result?;
                Ok(Restart::program(path))
            }
            Install::MacApp(app) => {
                let work = work_dir()?;
                let dmg = work.join(format!("slopp-{version}.dmg"));
                let result = get(&dmg, &mut progress).and_then(|()| {
                    progress(Progress::Unpacking {
                        version: version.clone(),
                    });
                    replace_app(app, &dmg, &work.join("mnt"))
                });
                let _ = std::fs::remove_dir_all(&work);
                result?;
                Ok(Restart {
                    program: "open".into(),
                    // A new instance, though this one hasn't quit yet.
                    args: vec!["-n".into(), app.into()],
                    installs: false,
                })
            }
            Install::Setup(_) => {
                let work = work_dir()?;
                let setup = work.join(format!("slopp-{version}-setup.exe"));
                if let Err(err) = get(&setup, &mut progress) {
                    let _ = std::fs::remove_file(&setup);
                    return Err(err);
                }
                Ok(Restart {
                    program: setup,
                    // Progress only, no questions; it closes what's left of
                    // this SLOPP and starts the new one.
                    args: [
                        "/SILENT",
                        "/SUPPRESSMSGBOXES",
                        "/NORESTART",
                        "/CLOSEAPPLICATIONS",
                        "/RELAUNCH=1",
                    ]
                    .map(OsString::from)
                    .to_vec(),
                    installs: true,
                })
            }
            Install::Folder(dir) => {
                let archive = dir.join(format!(".slopp-update-{suffix}"));
                let result = get(&archive, &mut progress).and_then(|()| {
                    progress(Progress::Unpacking {
                        version: version.clone(),
                    });
                    let names = programs();
                    let files = if suffix.ends_with(".zip") {
                        unzip(&archive, dir, &names)
                    } else {
                        untar(&archive, dir, &names)
                    }?;
                    for (new, dest) in &files {
                        replace_file(new, dest, cfg!(windows))?;
                    }
                    Ok(())
                });
                let _ = std::fs::remove_file(&archive);
                for name in programs() {
                    let _ = std::fs::remove_file(dir.join(format!("{name}.new")));
                }
                result?;
                Ok(Restart::program(&dir.join(programs()[0])))
            }
            Install::Elsewhere(_) => unreachable!("no download"),
        }
    }

    /// Removes what an earlier update left: the programs Windows couldn't
    /// delete while they ran, a downloaded installer, an interrupted
    /// download. Best effort.
    pub fn clean_up(&self) {
        if matches!(self, Install::MacApp(_) | Install::Setup(_)) {
            let _ = std::fs::remove_dir_all(update_dir());
        }
        match self {
            Install::AppImage(path) => {
                let _ = std::fs::remove_file(sibling(path, "update"));
            }
            Install::MacApp(app) => {
                let _ = std::fs::remove_dir_all(sibling(app, "old"));
                let _ = std::fs::remove_dir_all(sibling(app, "new"));
            }
            Install::Folder(dir) => {
                for name in programs() {
                    let _ = std::fs::remove_file(dir.join(format!("{name}.old")));
                }
            }
            Install::Setup(_) | Install::Elsewhere(_) => {}
        }
    }
}

/// Downloads `url` to `to`, passing on its progress, and checks its digest.
fn fetch(
    url: &str,
    sha256: &str,
    version: &str,
    to: &Path,
    progress: &mut dyn FnMut(Progress),
) -> anyhow::Result<()> {
    let mut shown = None;
    dbadmin::download(url, to, |fraction| {
        // Every hundredth, not every chunk.
        let step = fraction.map(|f| (f * 100.0) as u32);
        if shown != Some(step) {
            shown = Some(step);
            progress(Progress::Downloading {
                version: version.to_string(),
                fraction,
            });
        }
    })?;
    dbadmin::check_sha256(to, sha256)
}

/// What [`detect_from`] looks at, so tests can stand in for any system.
struct Probe<'a> {
    os: &'a str,
    arch: &'a str,
    debug: bool,
    exe: &'a Path,
    appimage: Option<&'a Path>,
    exists: &'a dyn Fn(&Path) -> bool,
    writable: &'a dyn Fn(&Path) -> bool,
}

fn detect_from(p: &Probe) -> Install {
    let built = matches!(
        (p.os, p.arch),
        ("linux", "x86_64") | ("macos", "aarch64") | ("windows", "x86_64")
    );
    if !built {
        return Install::Elsewhere(Elsewhere::NoBuild);
    }
    let in_folder = |dir: &Path, install: Install| {
        if (p.writable)(dir) {
            install
        } else {
            Install::Elsewhere(Elsewhere::ReadOnly(dir.to_path_buf()))
        }
    };
    if p.os == "linux"
        && let Some(appimage) = p.appimage
    {
        let dir = appimage.parent().unwrap_or(Path::new("/"));
        return in_folder(dir, Install::AppImage(appimage.to_path_buf()));
    }
    if p.exe.starts_with("/nix/store") {
        return Install::Elsewhere(Elsewhere::Nix);
    }
    let Some(dir) = p.exe.parent() else {
        return Install::Elsewhere(Elsewhere::Unknown);
    };
    // `cargo build`'s target/debug or target/release (or a target dir's).
    let in_target = matches!(
        dir.file_name().and_then(|n| n.to_str()),
        Some("debug" | "release")
    ) && dir
        .ancestors()
        .any(|a| a.file_name() == Some("target".as_ref()));
    if p.debug || in_target {
        return Install::Elsewhere(Elsewhere::Source);
    }
    match p.os {
        "macos" => {
            // SLOPP.app/Contents/MacOS/slopp
            let app = dir
                .parent()
                .filter(|_| dir.file_name() == Some("MacOS".as_ref()))
                .filter(|contents| contents.file_name() == Some("Contents".as_ref()))
                .and_then(Path::parent)
                .filter(|app| app.extension() == Some("app".as_ref()));
            let Some(app) = app else {
                return Install::Elsewhere(Elsewhere::Unknown);
            };
            if app
                .components()
                .any(|c| c.as_os_str() == "AppTranslocation")
            {
                return Install::Elsewhere(Elsewhere::Translocated);
            }
            in_folder(
                app.parent().unwrap_or(Path::new("/")),
                Install::MacApp(app.to_path_buf()),
            )
        }
        // The installer handles permissions itself (it asks for admin
        // rights for an install for all users).
        "windows" if (p.exists)(&dir.join("unins000.exe")) => Install::Setup(dir.to_path_buf()),
        "windows" => in_folder(dir, Install::Folder(dir.to_path_buf())),
        _ if (p.exists)(&dir.join("sloppctl")) => {
            in_folder(dir, Install::Folder(dir.to_path_buf()))
        }
        _ => Install::Elsewhere(Elsewhere::Unknown),
    }
}

/// Starts the new version after this one quits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restart {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// Running it installs the update (the Windows installer), rather than
    /// only starting what's installed already.
    pub installs: bool,
}

impl Restart {
    fn program(path: &Path) -> Restart {
        Restart {
            program: path.to_path_buf(),
            args: Vec::new(),
            installs: false,
        }
    }

    /// Runs the program through `launcher` (NixOS's `appimage-run`).
    pub fn through(mut self, launcher: &str) -> Restart {
        self.args.insert(0, self.program.into_os_string());
        self.program = launcher.into();
        self
    }

    pub fn spawn(&self) -> anyhow::Result<()> {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);
        // This AppImage's; the new one sets its own.
        for var in ["APPDIR", "APPIMAGE", "ARGV0", "OWD"] {
            cmd.env_remove(var);
        }
        crate::process::launch(cmd)?;
        Ok(())
    }
}

/// `slopp` and `sloppctl`, as the release names them here.
fn programs() -> [&'static str; 2] {
    if cfg!(windows) {
        ["slopp.exe", "sloppctl.exe"]
    } else {
        ["slopp", "sloppctl"]
    }
}

/// `.<name>.<what>` beside `path`.
fn sibling(path: &Path, what: &str) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.{what}"))
}

/// Where downloads that don't go beside SLOPP wait, in the user's own temp
/// folder (macOS and Windows).
fn update_dir() -> PathBuf {
    std::env::temp_dir().join("slopp-update")
}

fn work_dir() -> anyhow::Result<PathBuf> {
    let dir = update_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

/// Whether files can be created in `dir`.
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".slopp-write-test-{}", std::process::id()));
    let made = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .is_ok();
    if made {
        let _ = std::fs::remove_file(&probe);
    }
    made
}

fn make_executable(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("making {} executable", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Puts `new` at `dest`. On Windows a running program can't be replaced but
/// can be renamed, so the old one moves to `<name>.old` first.
fn replace_file(new: &Path, dest: &Path, windows: bool) -> anyhow::Result<()> {
    if windows && dest.exists() {
        let old = PathBuf::from(format!("{}.old", dest.display()));
        let _ = std::fs::remove_file(&old);
        std::fs::rename(dest, &old).with_context(|| format!("moving {} aside", dest.display()))?;
        if let Err(err) = std::fs::rename(new, dest) {
            let _ = std::fs::rename(&old, dest);
            return Err(err).with_context(|| format!("replacing {}", dest.display()));
        }
        return Ok(());
    }
    std::fs::rename(new, dest).with_context(|| format!("replacing {}", dest.display()))
}

/// The last part of an archive entry's name, whichever slash it uses.
fn base_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// Writes the archive's files named `names` to `<dir>/<name>.new`, and
/// returns each with where it goes. Fails unless all are there.
fn untar(archive: &Path, dir: &Path, names: &[&str]) -> anyhow::Result<Vec<(PathBuf, PathBuf)>> {
    let file =
        std::fs::File::open(archive).with_context(|| format!("reading {}", archive.display()))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut found = Vec::new();
    for entry in tar.entries().context("reading the archive")? {
        let mut entry = entry.context("reading the archive")?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().context("reading the archive")?;
        let Some(name) = path.to_str().map(base_name) else {
            continue;
        };
        let Some(&name) = names.iter().find(|&&n| n == name) else {
            continue;
        };
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .context("reading the archive")?;
        found.push(write_new(dir, name, &bytes)?);
    }
    all_found(found, names)
}

fn unzip(archive: &Path, dir: &Path, names: &[&str]) -> anyhow::Result<Vec<(PathBuf, PathBuf)>> {
    let file =
        std::fs::File::open(archive).with_context(|| format!("reading {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("reading the archive")?;
    let mut found = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).context("reading the archive")?;
        if !entry.is_file() {
            continue;
        }
        let entry_name = base_name(entry.name()).to_string();
        let Some(&name) = names.iter().find(|&&n| n == entry_name) else {
            continue;
        };
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .context("reading the archive")?;
        found.push(write_new(dir, name, &bytes)?);
    }
    all_found(found, names)
}

fn write_new(dir: &Path, name: &str, bytes: &[u8]) -> anyhow::Result<(PathBuf, PathBuf)> {
    let new = dir.join(format!("{name}.new"));
    std::fs::write(&new, bytes).with_context(|| format!("writing {}", new.display()))?;
    make_executable(&new)?;
    Ok((new, dir.join(name)))
}

fn all_found(
    found: Vec<(PathBuf, PathBuf)>,
    names: &[&str],
) -> anyhow::Result<Vec<(PathBuf, PathBuf)>> {
    for name in names {
        if !found
            .iter()
            .any(|(_, dest)| dest.file_name() == Some(name.as_ref()))
        {
            bail!("the download holds no {name}");
        }
    }
    Ok(found)
}

/// Copies the app out of the disk image beside `app` and swaps it in; the
/// old one is deleted (a running SLOPP keeps its open files).
fn replace_app(app: &Path, dmg: &Path, mnt: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(mnt).with_context(|| format!("creating {}", mnt.display()))?;
    let mut attach = crate::process::command("hdiutil");
    attach
        .args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-quiet"])
        .arg("-mountpoint")
        .arg(mnt)
        .arg(dmg);
    run(attach, "opening the disk image")?;
    let new = sibling(app, "new");
    let result = (|| {
        let source = mnt.join("SLOPP.app");
        if !source.is_dir() {
            bail!("the disk image holds no SLOPP.app");
        }
        let _ = std::fs::remove_dir_all(&new);
        // ditto keeps the bundle's attributes and signature intact.
        let mut copy = crate::process::command("ditto");
        copy.arg(&source).arg(&new);
        run(copy, "copying SLOPP.app")
    })();
    let mut detach = crate::process::command("hdiutil");
    detach.args(["detach", "-quiet", "-force"]).arg(mnt);
    let _ = run(detach, "closing the disk image");
    if let Err(err) = result {
        let _ = std::fs::remove_dir_all(&new);
        return Err(err);
    }
    let old = sibling(app, "old");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(app, &old).with_context(|| format!("moving {} aside", app.display()))?;
    if let Err(err) = std::fs::rename(&new, app) {
        let _ = std::fs::rename(&old, app);
        let _ = std::fs::remove_dir_all(&new);
        return Err(err).with_context(|| format!("replacing {}", app.display()));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

fn run(mut cmd: Command, what: &str) -> anyhow::Result<()> {
    let output = cmd.output().with_context(|| what.to_string())?;
    if !output.status.success() {
        let errors = crate::process::decode(&output.stderr);
        bail!("{what} failed: {}", errors.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_by_precedence() {
        assert!(is_newer("0.3.1", "0.3.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.0"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
        assert!(is_newer("0.4.0", "0.4.0-rc.1"));
        assert!(!is_newer("0.4.0-rc.1", "0.4.0"));
        assert!(is_newer("0.4.0-rc.2", "0.4.0-rc.1"));
        assert!(is_newer("0.4.0-rc.10", "0.4.0-rc.9"));
        assert!(is_newer("0.4.0-rc.1", "0.4.0-beta"));
        assert!(!is_newer("0.4.0+build.5", "0.4.0"));
        assert!(!is_newer("nonsense", "0.3.0"));
        assert!(!is_newer("0.4", "0.3.0"));
    }

    const RELEASE: &str = r#"{
        "html_url": "https://github.com/gabors0/slopp/releases/tag/v0.4.0",
        "tag_name": "v0.4.0",
        "assets": [
            {
                "name": "slopp-0.4.0-linux-x86_64.AppImage",
                "digest": "sha256:E28C9A0E",
                "browser_download_url": "https://github.com/gabors0/slopp/releases/download/v0.4.0/slopp-0.4.0-linux-x86_64.AppImage"
            },
            {
                "name": "slopp-0.4.0-windows-x86_64.zip",
                "digest": null,
                "browser_download_url": "https://github.com/gabors0/slopp/releases/download/v0.4.0/slopp-0.4.0-windows-x86_64.zip"
            }
        ]
    }"#;

    #[test]
    fn reads_the_release_and_insists_on_digests() {
        let release = parse_release(RELEASE).unwrap();
        assert_eq!(release.version, "0.4.0");
        assert_eq!(
            release.page,
            "https://github.com/gabors0/slopp/releases/tag/v0.4.0"
        );
        assert_eq!(
            release.asset("linux-x86_64.AppImage").unwrap(),
            (
                "https://github.com/gabors0/slopp/releases/download/v0.4.0/slopp-0.4.0-linux-x86_64.AppImage",
                "e28c9a0e"
            )
        );
        let err = release.asset("windows-x86_64.zip").unwrap_err();
        assert!(err.to_string().contains("no SHA-256"), "{err}");
        let err = release.asset("macos-arm64.dmg").unwrap_err();
        assert!(err.to_string().contains("no download"), "{err}");
    }

    fn detect(
        os: &str,
        exe: &str,
        appimage: Option<&str>,
        files: &[&str],
        read_only: &[&str],
    ) -> Install {
        detect_from(&Probe {
            os,
            arch: if os == "macos" { "aarch64" } else { "x86_64" },
            debug: false,
            exe: Path::new(exe),
            appimage: appimage.map(Path::new),
            exists: &|p| files.iter().any(|f| p == Path::new(f)),
            writable: &|p| !read_only.iter().any(|d| p == Path::new(d)),
        })
    }

    #[test]
    fn tells_how_it_was_installed() {
        let path = PathBuf::from;
        assert_eq!(
            detect(
                "linux",
                "/tmp/.mount_sloppX/usr/bin/slopp",
                Some("/home/u/.local/bin/slopp.AppImage"),
                &[],
                &[]
            ),
            Install::AppImage(path("/home/u/.local/bin/slopp.AppImage"))
        );
        assert_eq!(
            detect(
                "linux",
                "/tmp/.mount_sloppX/usr/bin/slopp",
                Some("/opt/slopp.AppImage"),
                &[],
                &["/opt"]
            ),
            Install::Elsewhere(Elsewhere::ReadOnly(path("/opt")))
        );
        assert_eq!(
            detect("linux", "/nix/store/abc-slopp/bin/slopp", None, &[], &[]),
            Install::Elsewhere(Elsewhere::Nix)
        );
        assert_eq!(
            detect(
                "linux",
                "/home/u/slopp/target/release/slopp",
                None,
                &["/home/u/slopp/target/release/sloppctl"],
                &[]
            ),
            Install::Elsewhere(Elsewhere::Source)
        );
        assert_eq!(
            detect(
                "linux",
                "/home/u/apps/slopp-0.3.0/slopp",
                None,
                &["/home/u/apps/slopp-0.3.0/sloppctl"],
                &[]
            ),
            Install::Folder(path("/home/u/apps/slopp-0.3.0"))
        );
        assert_eq!(
            detect("linux", "/usr/bin/slopp", None, &[], &[]),
            Install::Elsewhere(Elsewhere::Unknown)
        );
        assert_eq!(
            detect(
                "macos",
                "/Applications/SLOPP.app/Contents/MacOS/slopp",
                None,
                &[],
                &[]
            ),
            Install::MacApp(path("/Applications/SLOPP.app"))
        );
        assert_eq!(
            detect(
                "macos",
                "/private/var/folders/x/AppTranslocation/1234/d/SLOPP.app/Contents/MacOS/slopp",
                None,
                &[],
                &[]
            ),
            Install::Elsewhere(Elsewhere::Translocated)
        );
        assert_eq!(
            detect("macos", "/opt/homebrew/bin/slopp", None, &[], &[]),
            Install::Elsewhere(Elsewhere::Unknown)
        );
        assert_eq!(
            detect(
                "windows",
                "/Users/u/AppData/Local/Programs/SLOPP/slopp.exe",
                None,
                &["/Users/u/AppData/Local/Programs/SLOPP/unins000.exe"],
                &["/Users/u/AppData/Local/Programs/SLOPP"]
            ),
            Install::Setup(path("/Users/u/AppData/Local/Programs/SLOPP"))
        );
        assert_eq!(
            detect("windows", "/Users/u/Tools/slopp/slopp.exe", None, &[], &[]),
            Install::Folder(path("/Users/u/Tools/slopp"))
        );
        let no_build = detect_from(&Probe {
            os: "linux",
            arch: "aarch64",
            debug: false,
            exe: Path::new("/home/u/apps/slopp"),
            appimage: None,
            exists: &|_| true,
            writable: &|_| true,
        });
        assert_eq!(no_build, Install::Elsewhere(Elsewhere::NoBuild));
    }

    #[test]
    fn unpacks_both_programs_from_either_archive() {
        let dir = tempfile::tempdir().unwrap();
        let names = ["slopp", "sloppctl"];

        let tgz = dir.path().join("release.tar.gz");
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(&tgz).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gz);
        for (name, body) in [
            ("slopp-0.4.0/README.md", "readme"),
            ("slopp-0.4.0/slopp", "panel"),
            ("slopp-0.4.0/sloppctl", "ctl"),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, body.as_bytes()).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        let files = untar(&tgz, dir.path(), &names).unwrap();
        assert_eq!(files.len(), 2);
        for (new, dest) in &files {
            std::fs::write(dest, "old").unwrap();
            replace_file(new, dest, false).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("slopp")).unwrap(),
            "panel"
        );
        assert!(!dir.path().join("slopp.new").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = dir
                .path()
                .join("sloppctl")
                .metadata()
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
        }

        let zip_path = dir.path().join("release.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        zip.add_directory("slopp-0.4.0/", opts).unwrap();
        // Windows PowerShell 5.1's Compress-Archive writes backslashes.
        zip.start_file("slopp-0.4.0\\slopp", opts).unwrap();
        std::io::Write::write_all(&mut zip, b"new panel").unwrap();
        zip.finish().unwrap();
        let err = unzip(&zip_path, dir.path(), &names).unwrap_err();
        assert!(err.to_string().contains("no sloppctl"), "{err}");
    }

    #[test]
    fn moves_a_windows_program_aside_to_replace_it() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("slopp.exe");
        let new = dir.path().join("slopp.exe.new");
        std::fs::write(&dest, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        replace_file(&new, &dest, true).unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "new");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("slopp.exe.old")).unwrap(),
            "old"
        );
        Install::Folder(dir.path().to_path_buf()).clean_up();
        if cfg!(windows) {
            assert!(!dir.path().join("slopp.exe.old").exists());
        }
    }

    #[test]
    fn restarts_through_a_launcher() {
        let restart = Restart::program(Path::new("/home/u/slopp.AppImage")).through("appimage-run");
        assert_eq!(restart.program, PathBuf::from("appimage-run"));
        assert_eq!(restart.args, vec![OsString::from("/home/u/slopp.AppImage")]);
    }
}
