<table>
  <tr>
    <td align="center" valign="middle">
      <img src="https://raw.githubusercontent.com/gabors0/slopp/refs/heads/main/crates/slopp/assets/linux/org.slopp.Slopp.svg" width="120" alt="SLOPP logo">
    </td>
    <td>
      <h1>SLOPP</h1>
      An XAMPP-style local web stack with a control panel written in Rust on <a href="https://gpui-kit.com">GPUI</a>.
    </td>
  </tr>
</table>

The servers are the stock upstream programs, the
same modules XAMPP ships:

| Module  | What runs                                        |
| ------- | ------------------------------------------------ |
| Apache  | Apache httpd, with PHP through PHP-FPM or mod_php |
| MySQL   | MariaDB                                          |
| (admin) | phpMyAdmin and/or Adminer, downloaded on request |

Tomcat, ProFTPD/FileZilla and Mercury are not included.

SLOPP manages the servers the way the XAMPP control panel does: start and
stop each module, see its ports and PIDs, open its config and log files, open
the site, phpMyAdmin or Adminer, and spot port conflicts before they bite. The servers
keep running when the panel closes (unless you change that in Settings).

## Tray icon

On Linux SLOPP puts an icon in the system tray (StatusNotifierItem, shown by
KDE Plasma and most panels; GNOME needs the AppIndicator extension), on macOS
in the menu bar. Its top lamp is Apache, the middle one MySQL. The menu starts
and stops modules and opens the site, phpMyAdmin or Adminer; on Linux a left click
shows the panel.

With a tray, closing the window keeps SLOPP running there (Settings →
Startup turns that off); without one, closing the window quits. Launching
SLOPP again while it runs brings its window back instead of starting a
second copy, and so does a click on its Dock icon on macOS.
`SLOPP_HIDDEN=1 slopp` starts it in the tray only, e.g. for login autostart.

## Running it

On NixOS / with Nix:

```sh
nix run                      # the panel, with Apache, PHP and MariaDB on PATH
nix develop -c cargo run -p slopp
```

On other Linux distributions, install Apache, PHP-FPM (or PHP's Apache
module) and the MariaDB server with your package manager, then
`cargo run -p slopp`. SLOPP looks for them on `PATH` and in the usual
system locations; Settings → Programs takes explicit paths.

On macOS, with [Homebrew](https://brew.sh) and Xcode (GPUI compiles Metal
shaders):

```sh
brew install rust php mariadb     # mariadb@11.8 instead if you keep Homebrew's mysql
cargo run -p slopp
scripts/bundle-macos.sh           # or build target/release/SLOPP.app for /Applications
```

Apache comes with macOS; `brew install httpd` gets a newer one, which SLOPP
prefers and which runs PHP as mod_php like XAMPP (Apple's Apache only loads
signed modules, so PHP goes through PHP-FPM there). Homebrew's programs are
found wherever they are, linked or not, also when SLOPP is started from the
Dock without your shell's `PATH`. MacPorts' `/opt/local` is searched too.

Linux and macOS are supported. Windows is kept in mind (process handling and
file opening have per-platform paths) but untested. On macOS the Ports page
names only your own programs: a port taken by another user's (the system's
Apache, say) is still reported as in use, without a name.

## The stack folder

Everything lives in one folder, laid out like XAMPP's
(default `~/.local/share/slopp`, also on macOS, or `$SLOPP_ROOT`; MariaDB's
installer needs a path without spaces):

```
htdocs/                      your sites (a welcome page on first run)
apache/conf/httpd.conf       Apache config (yours to edit)
apache/conf/extra/           httpd-ssl.conf, httpd-slopp.conf (PHP, admin tools)
apache/logs/                 error.log, access.log
php/php.ini                  PHP config, php/logs/php_error_log
mysql/my.cnf                 MariaDB config, mysql/data/ the databases
phpMyAdmin/                  after "Get phpMyAdmin" (MySQL's Admin menu)
adminer/                     after "Get Adminer": adminer.php, index.php and adminer.css to edit
tmp/  run/                   sockets, sessions, pid files
```

Config files are written once and never overwritten. Ports and
machine-specific paths (Apache's module directory, PHP-FPM's socket) are
passed on the command line at start, so changing ports in Settings doesn't
touch your edits. The one exception: installing Adminer into a stack whose
`httpd-slopp.conf` predates it appends the `/adminer` alias (restart Apache
afterwards).

phpMyAdmin gets the dark BooDark theme (orange, like the panel) as its
default; its home page still offers the stock themes. Adminer gets a design
in the panel's colours that follows the system's light or dark mode; delete
`adminer/adminer.css` for Adminer's own look. "Update" in MySQL's Admin menu
fetches the latest release and keeps your config.

Like XAMPP, MariaDB's `root` user has no password and phpMyAdmin and Adminer
log in as root. Both only listen on localhost; don't expose this stack to a network.

Ports default to XAMPP's 80/443/3306 where the user may bind them (always on
macOS), otherwise 8080/8443/3306.

## From a terminal

`sloppctl` is the `lampp` script's counterpart:

```sh
sloppctl start [apache|mysql]
sloppctl stop [apache|mysql]
sloppctl restart [apache|mysql]
sloppctl status
sloppctl ports               # who listens where
sloppctl paths               # where the stack and programs are
sloppctl install-phpmyadmin
sloppctl install-adminer
```

## Layout

- `crates/slopp-core`: no GUI. Finds the programs (`toolchain`), writes the
  stack's config (`setup`, `templates`), starts/stops/watches the servers
  (`stack`, `process`), lists listening ports (`net`), installs phpMyAdmin and Adminer (`dbadmin`),
  and the `sloppctl` binary.
- `crates/slopp`: the GPUI panel and the tray icon.
- `scripts/bundle-macos.sh`: builds `SLOPP.app`.

Settings are stored in `~/.config/slopp/settings.json` (`$XDG_CONFIG_HOME`
is honoured on macOS as well).

Fonts: IBM Plex Sans and IBM Plex Mono (SIL Open Font License).
