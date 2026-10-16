//! User settings: ports, where the stack root is, binary overrides, the
//! editor and terminal to open things with, which modules start with the
//! panel, the panel's appearance and window style, and whether it looks for updates. Saved as JSON in the config directory.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::Module;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ports {
    pub http: u16,
    pub https: u16,
    pub mysql: u16,
    /// Mailpit's SMTP server, where PHP's mail goes.
    pub smtp: u16,
    /// Mailpit's web page.
    pub mailpit: u16,
}

impl Default for Ports {
    /// XAMPP's ports where this user may bind them, otherwise 8080/8443.
    fn default() -> Self {
        let low = can_bind_low_ports();
        Self {
            http: if low { 80 } else { 8080 },
            https: if low { 443 } else { 8443 },
            mysql: 3306,
            smtp: 1025,
            mailpit: 8025,
        }
    }
}

/// Whether ports below 1024 are open to this process (root, or a lowered
/// `ip_unprivileged_port_start`).
fn can_bind_low_ports() -> bool {
    #[cfg(target_os = "linux")]
    {
        if process_is_root() {
            return true;
        }
        std::fs::read_to_string("/proc/sys/net/ipv4/ip_unprivileged_port_start")
            .ok()
            .and_then(|s| s.trim().parse::<u16>().ok())
            .is_some_and(|start| start <= 80)
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        // macOS lets anyone bind low ports since Mojave.
        cfg!(target_os = "macos") || process_is_root()
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(unix)]
fn process_is_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

/// Paths that replace what is found on `PATH`. Empty means "find it".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Binaries {
    pub httpd: Option<PathBuf>,
    /// Apache's modules directory (`mod_*.so`).
    pub apache_modules: Option<PathBuf>,
    pub php_fpm: Option<PathBuf>,
    /// `libphp.so`; when set PHP runs inside Apache instead of PHP-FPM.
    pub php_module: Option<PathBuf>,
    pub mariadbd: Option<PathBuf>,
    pub mariadb_install_db: Option<PathBuf>,
    pub mariadb_client: Option<PathBuf>,
    pub mailpit: Option<PathBuf>,
}

/// The panel's colour scheme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    /// Follows the system's light or dark mode.
    #[default]
    System,
    Dark,
    Light,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Dark, Appearance::Light];

    pub fn name(self) -> &'static str {
        match self {
            Appearance::System => "System",
            Appearance::Dark => "Dark",
            Appearance::Light => "Light",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The stack root; `None` is [`crate::Layout::default_root`].
    pub root: Option<PathBuf>,
    pub ports: Ports,
    pub binaries: Binaries,
    /// Command that opens config and log files, e.g. `code` or `kate`.
    /// `None` uses the desktop's default application.
    pub editor: Option<String>,
    /// Terminal for the Shell button, e.g. `konsole` or `kitty`.
    /// `None` picks one that is installed.
    pub terminal: Option<String>,
    /// Modules started when the panel opens.
    pub autostart: Vec<Module>,
    /// Stop running modules when the panel quits (XAMPP leaves them running).
    pub stop_on_exit: bool,
    /// Keep running in the system tray when the window closes.
    pub close_to_tray: bool,
    pub appearance: Appearance,
    /// Show the panel as a small window with just the modules.
    pub compact: bool,
    /// Look for a new SLOPP release on GitHub when the panel opens, and
    /// daily while it runs.
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            root: None,
            ports: Ports::default(),
            binaries: Binaries::default(),
            editor: None,
            terminal: None,
            autostart: Vec::new(),
            stop_on_exit: false,
            close_to_tray: true,
            appearance: Appearance::System,
            compact: false,
            check_updates: true,
        }
    }
}

impl Settings {
    /// `slopp/settings.json` in the config directory (`~/.config` unless
    /// `$XDG_CONFIG_HOME` says otherwise).
    pub fn default_path() -> PathBuf {
        crate::layout::base_dir("XDG_CONFIG_HOME", ".config", dirs::config_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("slopp")
            .join("settings.json")
    }

    /// Reads settings, falling back to defaults when the file is missing.
    pub fn load(path: &Path) -> anyhow::Result<Settings> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("{} is not valid settings JSON", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(self)?;
        // Write then rename so a crash never leaves half a file.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    pub fn root(&self) -> PathBuf {
        self.root
            .clone()
            .unwrap_or_else(crate::Layout::default_root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::load(&dir.path().join("nope.json")).unwrap();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn round_trips_and_tolerates_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/settings.json");
        let mut settings = Settings::default();
        settings.ports.mysql = 3307;
        settings.autostart = vec![Module::MySql];
        settings.appearance = Appearance::Light;
        settings.compact = true;
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), settings);

        std::fs::write(&path, r#"{"ports":{"http":81}}"#).unwrap();
        let partial = Settings::load(&path).unwrap();
        assert_eq!(partial.ports.http, 81);
        assert_eq!(partial.ports.mysql, 3306);
        assert_eq!(partial.ports.smtp, 1025);
        assert!(partial.close_to_tray, "missing fields take the defaults");
        assert_eq!(partial.appearance, Appearance::System);
        assert!(!partial.compact);
        assert!(partial.check_updates);

        std::fs::write(&path, r#"{"appearance":"dark"}"#).unwrap();
        assert_eq!(Settings::load(&path).unwrap().appearance, Appearance::Dark);
    }
}
