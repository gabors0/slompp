//! Desktop integration for the AppImage. Wayland taskbars and docks find a
//! window's icon through the desktop file named after its app id
//! (`org.slopp.Slopp`), which an AppImage doesn't install, so on each launch
//! from one SLOPP writes its own into `~/.local/share/applications`, with the
//! icon beside it in the hicolor theme, pointing at the AppImage. A moved
//! AppImage updates them on its next launch.
//!
//! There's no `TryExec`: GLib and KDE drop an entry whose `TryExec` isn't
//! executable, and NixOS's `appimage-run` runs AppImages that aren't. When
//! that started SLOPP, the entry starts it the same way, since NixOS can't
//! run an AppImage by itself.
//!
//! Skipped when the desktop already knows SLOPP: a desktop file of that name
//! that SLOPP didn't write (a package, or one the user wrote), or one that
//! AppImageLauncher, Gear Lever and the like wrote for this AppImage. Also
//! skipped when `~/.local/share/appimagekit/no_desktopintegration` exists,
//! the AppImage convention for opting out.

use std::path::{Path, PathBuf};
use std::{fs, io};

const APP_ID: &str = "org.slopp.Slopp";
/// Marks the desktop files this module writes, so it only replaces its own.
const MARKER: &str = "X-SLOPP-AppImage=true";
const ICON: &str = include_str!("../assets/linux/org.slopp.Slopp.svg");
const DESKTOP: &str = include_str!("../assets/linux/org.slopp.Slopp.desktop");

/// Integrates the AppImage SLOPP runs from, if any, on a thread of its own.
pub fn integrate() {
    let Some(appimage) = std::env::var_os("APPIMAGE").map(PathBuf::from) else {
        return;
    };
    let Some(data) = dirs::data_dir() else {
        return;
    };
    std::thread::spawn(move || {
        let launcher = launcher();
        if let Err(err) = integrate_into(&appimage, launcher, &data, &system_data_dirs()) {
            tracing::warn!("couldn't add SLOPP to the desktop's applications: {err}");
        }
    });
}

/// `appimage-run` if it started this AppImage (it unpacks them into its
/// cache folder, `$APPDIR`) and is on `PATH`. Without it on `PATH` NixOS is
/// probably set up to run AppImages directly (`programs.appimage.binfmt`).
pub fn launcher() -> Option<&'static str> {
    const APPIMAGE_RUN: &str = "appimage-run";
    let appdir = PathBuf::from(std::env::var_os("APPDIR")?);
    let unpacked_by_it = appdir.components().any(|c| c.as_os_str() == APPIMAGE_RUN);
    let path = std::env::var_os("PATH")?;
    let on_path = std::env::split_paths(&path).any(|dir| dir.join(APPIMAGE_RUN).is_file());
    (unpacked_by_it && on_path).then_some(APPIMAGE_RUN)
}

fn system_data_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    std::env::split_paths(&dirs).collect()
}

fn integrate_into(
    appimage: &Path,
    launcher: Option<&str>,
    data: &Path,
    system: &[PathBuf],
) -> io::Result<()> {
    if data.join("appimagekit/no_desktopintegration").exists() {
        return Ok(());
    }
    let Some(entry) = appimage
        .to_str()
        .and_then(|path| desktop_entry(path, launcher))
    else {
        return Ok(());
    };
    let applications = data.join("applications");
    let desktop_file = applications.join(format!("{APP_ID}.desktop"));
    match fs::read_to_string(&desktop_file) {
        Ok(existing) if !existing.lines().any(|line| line == MARKER) => return Ok(()),
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    let installed = system.iter().any(|dir| {
        dir.join("applications")
            .join(format!("{APP_ID}.desktop"))
            .exists()
    });
    if installed || integrated_elsewhere(&applications, &desktop_file, appimage) {
        // Leave it to the other desktop file, and drop ours if we wrote one
        // earlier, so the menu doesn't list SLOPP twice.
        return match fs::remove_file(&desktop_file) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        };
    }

    let icon = data
        .join("icons/hicolor/scalable/apps")
        .join(format!("{APP_ID}.svg"));
    write_if_changed(&icon, ICON)?;
    write_if_changed(&desktop_file, &entry)
}

/// Whether another desktop file in `applications` launches this AppImage.
fn integrated_elsewhere(applications: &Path, ours: &Path, appimage: &Path) -> bool {
    let Ok(entries) = fs::read_dir(applications) else {
        return false;
    };
    let Some(appimage) = appimage.to_str() else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        path != ours
            && path.extension().is_some_and(|ext| ext == "desktop")
            && fs::read_to_string(&path).is_ok_and(|text| {
                text.lines()
                    .any(|line| line.starts_with("Exec=") && line.contains(appimage))
            })
    })
}

/// The bundled desktop file, launching the AppImage at `path`, through
/// `launcher` if given.
fn desktop_entry(path: &str, launcher: Option<&str>) -> Option<String> {
    let quoted = exec_quote(path)?;
    let exec = match launcher {
        Some(launcher) => format!("{launcher} {quoted}"),
        None => quoted,
    };
    let mut out = String::new();
    for line in DESKTOP.lines() {
        if line.starts_with("Exec=") {
            out.push_str(&format!("Exec={exec}\n"));
        } else if !line.starts_with("TryExec=") {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(MARKER);
    out.push('\n');
    Some(out)
}

/// `path` as a quoted argument of a desktop file's `Exec` key, or `None` for
/// a path the format can't hold. The value is unescaped once as a string
/// (`\\` is a backslash) and then as a quoted argument (`\"`, `` \` ``, `\$`
/// and `\\`), and `%` starts a field code.
fn exec_quote(path: &str) -> Option<String> {
    if path.chars().any(char::is_control) {
        return None;
    }
    let mut out = String::from("\"");
    for c in path.chars() {
        match c {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '\\' => out.push_str("\\\\\\\\"),
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    Some(out)
}

fn write_if_changed(path: &Path, contents: &str) -> io::Result<()> {
    if fs::read_to_string(path).is_ok_and(|existing| existing == contents) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // Written beside it and renamed, so a desktop watching the folder never
    // reads half a file.
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_exec_paths() {
        assert_eq!(
            exec_quote("/home/me/Apps/SLOPP.AppImage").unwrap(),
            r#""/home/me/Apps/SLOPP.AppImage""#
        );
        assert_eq!(
            exec_quote(r#"/a b/100% "$x`\.AppImage"#).unwrap(),
            r#""/a b/100%% \\"\\$x\\`\\\\.AppImage""#
        );
        assert_eq!(exec_quote("/a\nb"), None);
    }

    #[test]
    fn desktop_entry_launches_the_appimage() {
        let entry = desktop_entry("/my apps/SLOPP.AppImage", None).unwrap();
        assert!(entry.contains("\nExec=\"/my apps/SLOPP.AppImage\"\n"));
        assert!(entry.contains("\nIcon=org.slopp.Slopp\n"));
        assert!(entry.contains("\nStartupWMClass=org.slopp.Slopp\n"));
        assert!(entry.ends_with("X-SLOPP-AppImage=true\n"));
        // Desktops drop entries whose TryExec isn't executable, which an
        // AppImage started with appimage-run needn't be.
        assert!(!entry.contains("TryExec"));
        assert_eq!(entry.matches("Exec=").count(), 1);

        let entry = desktop_entry("/apps/SLOPP.AppImage", Some("appimage-run")).unwrap();
        assert!(entry.contains("\nExec=appimage-run \"/apps/SLOPP.AppImage\"\n"));
    }

    #[test]
    fn integrates_and_follows_a_moved_appimage() {
        let data = tempfile::tempdir().unwrap();
        let desktop = data.path().join("applications/org.slopp.Slopp.desktop");
        let icon = data
            .path()
            .join("icons/hicolor/scalable/apps/org.slopp.Slopp.svg");

        integrate_into(Path::new("/old/SLOPP.AppImage"), None, data.path(), &[]).unwrap();
        assert!(
            fs::read_to_string(&desktop)
                .unwrap()
                .contains("Exec=\"/old/SLOPP.AppImage\"")
        );
        assert_eq!(fs::read_to_string(&icon).unwrap(), ICON);

        integrate_into(Path::new("/new/SLOPP.AppImage"), None, data.path(), &[]).unwrap();
        let text = fs::read_to_string(&desktop).unwrap();
        assert!(text.contains("Exec=\"/new/SLOPP.AppImage\""));
        assert!(!text.contains("/old/"));
    }

    #[test]
    fn leaves_other_desktop_files_alone() {
        let data = tempfile::tempdir().unwrap();
        let applications = data.path().join("applications");
        fs::create_dir_all(&applications).unwrap();
        let desktop = applications.join("org.slopp.Slopp.desktop");
        fs::write(&desktop, "[Desktop Entry]\nExec=slopp\n").unwrap();

        integrate_into(Path::new("/apps/SLOPP.AppImage"), None, data.path(), &[]).unwrap();
        assert_eq!(
            fs::read_to_string(&desktop).unwrap(),
            "[Desktop Entry]\nExec=slopp\n"
        );
    }

    #[test]
    fn steps_aside_for_other_integrations() {
        let data = tempfile::tempdir().unwrap();
        let applications = data.path().join("applications");
        let ours = applications.join("org.slopp.Slopp.desktop");
        let appimage = Path::new("/apps/SLOPP.AppImage");
        integrate_into(appimage, None, data.path(), &[]).unwrap();
        assert!(ours.exists());

        // AppImageLauncher's copy of the bundled desktop file.
        fs::write(
            applications.join("appimagekit_0123-SLOPP.desktop"),
            "[Desktop Entry]\nExec=/apps/SLOPP.AppImage\n",
        )
        .unwrap();
        integrate_into(appimage, None, data.path(), &[]).unwrap();
        assert!(!ours.exists());

        // Installed system-wide, e.g. from a package.
        let system = tempfile::tempdir().unwrap();
        fs::create_dir_all(system.path().join("applications")).unwrap();
        fs::write(
            system.path().join("applications/org.slopp.Slopp.desktop"),
            "",
        )
        .unwrap();
        let other = tempfile::tempdir().unwrap();
        integrate_into(appimage, None, other.path(), &[system.path().to_owned()]).unwrap();
        assert!(
            !other
                .path()
                .join("applications/org.slopp.Slopp.desktop")
                .exists()
        );
    }

    #[test]
    fn honours_the_opt_out() {
        let data = tempfile::tempdir().unwrap();
        fs::create_dir_all(data.path().join("appimagekit")).unwrap();
        fs::write(data.path().join("appimagekit/no_desktopintegration"), "").unwrap();
        integrate_into(Path::new("/apps/SLOPP.AppImage"), None, data.path(), &[]).unwrap();
        assert!(!data.path().join("applications").exists());
    }
}
