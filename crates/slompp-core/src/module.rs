use serde::{Deserialize, Serialize};

/// A server the control panel manages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Module {
    /// Apache httpd with PHP (as `mod_php` or through PHP-FPM).
    Apache,
    /// MariaDB (XAMPP calls it MySQL).
    #[serde(rename = "mysql")]
    MySql,
}

impl Module {
    pub const ALL: [Module; 2] = [Module::Apache, Module::MySql];

    /// The name XAMPP shows.
    pub fn name(self) -> &'static str {
        match self {
            Module::Apache => "Apache",
            Module::MySql => "MySQL",
        }
    }

    /// Lowercase identifier for files and the command line.
    pub fn id(self) -> &'static str {
        match self {
            Module::Apache => "apache",
            Module::MySql => "mysql",
        }
    }

    pub fn from_id(id: &str) -> Option<Module> {
        Module::ALL
            .into_iter()
            .find(|m| m.id().eq_ignore_ascii_case(id))
    }
}

impl std::fmt::Display for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}
