//! Installing the servers on Linux with the distribution's package manager
//! (apt, dnf or pacman), run as root through `pkexec`, which asks
//! for the password with the desktop's polkit agent (or in the terminal).
//! One `pkexec` runs a small fixed script, so there is one password prompt.
//!
//! Debian and Ubuntu start and enable a server's service when its package
//! goes in: `apache2` would take port 80 and `mariadb` port 3306, MySQL's
//! default. The script stops and disables the services of the packages it
//! newly installed; the other distributions leave services off.
//!
//! openSUSE isn't among them yet: its `httpd` is a wrapper script around
//! one binary per MPM, with the modules in MPM-specific directories and PHP's
//! as `mod_php8.so`, which [`Toolchain::detect`](crate::Toolchain::detect)
//! doesn't find.
//!
//! NixOS and image-based systems (Fedora Atomic, SteamOS) can't install
//! packages this way, so SLOPP says what to do there instead.

use std::path::PathBuf;
use std::process::Command;

use anyhow::bail;

use crate::install::{self, Package, Unavailable};

/// A family of distributions sharing a package manager and package names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Debian,
    Fedora,
    Arch,
}

/// What `/etc/os-release` says about the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum System {
    Family(Family),
    NixOs,
    /// Packages are part of a read-only image.
    Image,
    Unknown,
}

impl Family {
    /// The package manager's program, and its name.
    fn manager(self) -> (&'static str, &'static str) {
        match self {
            Family::Debian => ("/usr/bin/apt-get", "apt"),
            Family::Fedora => ("/usr/bin/dnf", "dnf"),
            Family::Arch => ("/usr/bin/pacman", "pacman"),
        }
    }

    /// The packages for `packages`. PHP comes as the Apache module, the way
    /// SLOPP prefers it, where the distribution has one (Fedora's PHP runs
    /// as PHP-FPM), plus mysqli and mbstring for phpMyAdmin.
    fn names(self, packages: &[Package]) -> Vec<&'static str> {
        let mut names = Vec::new();
        for package in packages {
            let these: &[&str] = match (self, package) {
                (Family::Debian, Package::Apache) => &["apache2"],
                (Family::Debian, Package::Php) => {
                    &["libapache2-mod-php", "php-mysql", "php-mbstring"]
                }
                (Family::Debian, Package::MariaDb) => &["mariadb-server"],
                (Family::Fedora, Package::Apache) => &["httpd", "mod_ssl"],
                (Family::Fedora, Package::Php) => {
                    &["php", "php-fpm", "php-mysqlnd", "php-mbstring"]
                }
                (Family::Fedora, Package::MariaDb) => &["mariadb-server"],
                (Family::Arch, Package::Apache) => &["apache"],
                (Family::Arch, Package::Php) => &["php", "php-apache"],
                (Family::Arch, Package::MariaDb) => &["mariadb"],
                // Not packaged by the distributions; downloaded instead.
                (_, Package::Mailpit) => &[],
            };
            for name in these {
                if !names.contains(name) {
                    names.push(name);
                }
            }
        }
        names
    }

    /// The command that installs `names`, run as root.
    fn command(self, names: &[&str]) -> String {
        let names = names.join(" ");
        match self {
            Family::Debian => format!("apt-get install -y {names}"),
            Family::Fedora => format!("dnf install -y {names}"),
            Family::Arch => format!("pacman -S --needed --noconfirm {names}"),
        }
    }

    /// The script `pkexec` runs: the install, and on Debian stopping the
    /// services it brought.
    fn script(self, names: &[&str]) -> String {
        let install = self.command(names);
        match self {
            Family::Debian => format!(
                r#"exec 2>&1
export DEBIAN_FRONTEND=noninteractive
new=
for pair in apache2:apache2 mariadb-server:mariadb; do
  dpkg-query -W -f='${{Status}}' "${{pair%%:*}}" 2>/dev/null | grep -q 'ok installed' || new="$new ${{pair#*:}}"
done
apt-get -o DPkg::Lock::Timeout=120 update || true
apt-get -o DPkg::Lock::Timeout=120 {rest} || exit
for unit in $new; do
  if systemctl is-enabled --quiet "$unit" 2>/dev/null || systemctl is-active --quiet "$unit" 2>/dev/null; then
    echo "Stopping the system's $unit service: SLOPP runs its own"
    systemctl disable --now "$unit" || true
  fi
done
"#,
                rest = install.trim_start_matches("apt-get ")
            ),
            _ => format!("exec 2>&1\n{install}\n"),
        }
    }
}

/// What installs the servers here: the family's package manager, as root.
#[derive(Debug, Clone)]
pub struct Installer {
    family: Family,
    /// `None` when SLOPP already runs as root.
    pkexec: Option<PathBuf>,
}

/// `pkexec`'s exit codes when the password prompt was dismissed, and when
/// it couldn't ask (no polkit agent) or the user isn't allowed.
const PKEXEC_DISMISSED: i32 = 126;
const PKEXEC_NOT_AUTHORIZED: i32 = 127;

impl Installer {
    pub fn find(packages: &[Package]) -> Result<Installer, Unavailable> {
        let os_release = std::fs::read_to_string("/etc/os-release")
            .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
            .unwrap_or_default();
        let mut system = classify(&os_release);
        if std::path::Path::new("/run/ostree-booted").exists() {
            system = System::Image;
        }
        let family = match system {
            System::Family(family) => family,
            System::NixOs => {
                let attrs: Vec<&str> = packages
                    .iter()
                    .map(|p| match p {
                        Package::Apache => "nixpkgs#apacheHttpd",
                        Package::Php => "nixpkgs#php",
                        Package::MariaDb => "nixpkgs#mariadb",
                        Package::Mailpit => "nixpkgs#mailpit",
                    })
                    .collect();
                return Err(Unavailable {
                    message: format!(
                        "On NixOS, add {} to your configuration's packages, or run \
                         `nix profile install {}`, then restart SLOPP.",
                        install::names(packages),
                        attrs.join(" ")
                    ),
                    notice: "Install the servers with Nix: see the log".into(),
                });
            }
            System::Image => {
                return Err(Unavailable {
                    message: format!(
                        "This system's packages are part of its image: layer {} onto it \
                         (with rpm-ostree on Fedora Atomic), or set the programs' paths in \
                         Settings.",
                        install::names(packages)
                    ),
                    notice: "Can't install packages on this system: see the log".into(),
                });
            }
            System::Unknown => {
                return Err(Unavailable {
                    message: format!(
                        "SLOPP doesn't know this distribution's package manager: install {} \
                         with it (PHP with its Apache module or PHP-FPM, MariaDB's server), \
                         then restart SLOPP.",
                        install::names(packages)
                    ),
                    notice: "Install the servers with your package manager".into(),
                });
            }
        };
        let (manager, name) = family.manager();
        let command = family.command(&family.names(packages));
        if install::first_program(&[manager]).is_none() {
            return Err(Unavailable {
                message: format!(
                    "Installing the servers needs {name}, which isn't at {manager}. \
                     Install {} yourself.",
                    install::names(packages)
                ),
                notice: format!("{name} isn't installed"),
            });
        }
        if is_root() {
            return Ok(Installer {
                family,
                pkexec: None,
            });
        }
        match install::first_program(&["/usr/bin/pkexec", "/bin/pkexec"]) {
            Some(pkexec) => Ok(Installer {
                family,
                pkexec: Some(pkexec),
            }),
            None => Err(Unavailable {
                message: format!(
                    "Installing the servers needs pkexec (polkit) to ask for your password. \
                     Install from a terminal instead: sudo {command}"
                ),
                notice: "pkexec isn't installed: see the log".into(),
            }),
        }
    }

    /// The package manager's name: "apt".
    pub fn name(&self) -> &'static str {
        self.family.manager().1
    }

    /// Installs `packages`, passing each line the package manager prints to
    /// `progress`.
    pub fn install(
        &self,
        packages: &[Package],
        mut progress: impl FnMut(String),
    ) -> anyhow::Result<()> {
        let names = self.family.names(packages);
        let script = self.family.script(&names);
        let mut cmd = match &self.pkexec {
            Some(pkexec) => {
                progress("Asking for your password…".into());
                let mut cmd = Command::new(pkexec);
                cmd.arg("/bin/sh");
                cmd
            }
            None => Command::new("/bin/sh"),
        };
        cmd.arg("-c").arg(script);
        let outcome = install::run(cmd, &mut progress)?;
        let name = self.name();
        match outcome.code {
            Some(0) => Ok(()),
            Some(PKEXEC_DISMISSED) if self.pkexec.is_some() => {
                bail!("the password prompt was dismissed")
            }
            Some(PKEXEC_NOT_AUTHORIZED) if self.pkexec.is_some() => bail!(
                "pkexec couldn't ask for your password (is a polkit agent running?). \
                 Install from a terminal instead: sudo {}",
                self.family.command(&names)
            ),
            code => {
                let detail = outcome.detail();
                let code = code.map_or("no exit code".into(), |c| format!("exit code {c}"));
                if detail.is_empty() {
                    bail!("{name} failed ({code})");
                }
                bail!("{name} failed ({code}): {detail}")
            }
        }
    }
}

fn is_root() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: geteuid has no preconditions.
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Reads `ID` and `ID_LIKE` from os-release.
fn classify(os_release: &str) -> System {
    let value = |key: &str| {
        os_release
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches(['"', '\'']).to_ascii_lowercase())
            .unwrap_or_default()
    };
    let id = value("ID");
    let like = value("ID_LIKE");
    match id.as_str() {
        "nixos" => return System::NixOs,
        "steamos" => return System::Image,
        _ => {}
    }
    for word in std::iter::once(id.as_str()).chain(like.split_whitespace()) {
        let family = match word {
            "debian" | "ubuntu" => Family::Debian,
            "fedora" | "rhel" | "centos" => Family::Fedora,
            "arch" => Family::Arch,
            _ => continue,
        };
        return System::Family(family);
    }
    System::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_distributions() {
        let cases = [
            (
                "ID=ubuntu\nID_LIKE=debian\n",
                System::Family(Family::Debian),
            ),
            (
                "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n",
                System::Family(Family::Debian),
            ),
            ("ID=fedora\n", System::Family(Family::Fedora)),
            (
                "ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n",
                System::Family(Family::Fedora),
            ),
            (
                "ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n",
                System::Unknown,
            ),
            ("ID=cachyos\nID_LIKE=arch\n", System::Family(Family::Arch)),
            ("ID=steamos\nID_LIKE=arch\n", System::Image),
            ("ID=nixos\n", System::NixOs),
            ("ID=gentoo\n", System::Unknown),
            ("", System::Unknown),
        ];
        for (text, system) in cases {
            assert_eq!(classify(text), system, "{text}");
        }
    }

    #[test]
    fn names_each_package_once() {
        assert_eq!(
            Family::Debian.names(&Package::ALL),
            [
                "apache2",
                "libapache2-mod-php",
                "php-mysql",
                "php-mbstring",
                "mariadb-server"
            ]
        );
        assert_eq!(
            Family::Arch.names(&[Package::Php, Package::Php]),
            ["php", "php-apache"]
        );
    }

    #[test]
    fn debian_stops_the_services_it_brought() {
        let script = Family::Debian.script(&["apache2", "mariadb-server"]);
        assert!(script.contains(
            "apt-get -o DPkg::Lock::Timeout=120 install -y apache2 mariadb-server || exit\n"
        ));
        assert!(script.contains("for pair in apache2:apache2 mariadb-server:mariadb; do"));
        assert!(script.contains("-f='${Status}' \"${pair%%:*}\""));
        assert!(script.contains("systemctl disable --now \"$unit\""));
        assert_eq!(
            Family::Arch.script(&["php"]),
            "exec 2>&1\npacman -S --needed --noconfirm php\n"
        );
    }
}
