# SLOMPP

An XAMPP-style local web stack with a control panel written in Rust on
[GPUI](https://gpui-kit.com). The servers are the stock upstream programs, the
same modules XAMPP ships:

| Module  | What runs                                        |
| ------- | ------------------------------------------------ |
| Apache  | Apache httpd, with PHP through PHP-FPM or mod_php |
| MySQL   | MariaDB                                          |
| (admin) | phpMyAdmin and/or Adminer, downloaded on request |

Tomcat, ProFTPD/FileZilla and Mercury are not included.

SLOMPP manages the servers the way the XAMPP control panel does: start and
stop each module, see its ports and PIDs, open its config and log files, open
the site, phpMyAdmin or Adminer, and spot port conflicts before they bite. The servers
keep running when the panel closes (unless you change that in Settings).

## Tray icon

On Linux SLOMPP puts an icon in the system tray (StatusNotifierItem, shown by
KDE Plasma and most panels; GNOME needs the AppIndicator extension), on macOS
in the menu bar. Its top lamp is Apache, the middle one MySQL. The menu starts
and stops modules and opens the site, phpMyAdmin or Adminer; on Linux a left click
shows the panel.

With a tray, closing the window keeps SLOMPP running there (Settings →
Startup turns that off); without one, closing the window quits. Launching
SLOMPP again while it runs brings its window back instead of starting a
second copy, and so does a click on its Dock icon on macOS.
`SLOMPP_HIDDEN=1 slompp` starts it in the tray only, e.g. for login autostart.

## Running it

On NixOS / with Nix:

```sh
nix run                      # the panel, with Apache, PHP and MariaDB on PATH
nix develop -c cargo run -p slompp
```

On other Linux distributions, install Apache, PHP-FPM (or PHP's Apache
module) and the MariaDB server with your package manager, then
`cargo run -p slompp`. SLOMPP looks for them on `PATH` and in the usual
system locations; Settings → Programs takes explicit paths.

On macOS, with [Homebrew](https://brew.sh) and Xcode (GPUI compiles Metal
shaders):

```sh
brew install rust php mariadb     # mariadb@11.8 instead if you keep Homebrew's mysql
cargo run -p slompp
scripts/bundle-macos.sh           # or build target/release/SLOMPP.app for /Applications
```

Apache comes with macOS; `brew install httpd` gets a newer one, which SLOMPP
prefers and which runs PHP as mod_php like XAMPP (Apple's Apache only loads
signed modules, so PHP goes through PHP-FPM there). Homebrew's programs are
found wherever they are, linked or not, also when SLOMPP is started from the
Dock without your shell's `PATH`. MacPorts' `/opt/local` is searched too.

Linux and macOS are supported. Windows is kept in mind (process handling and
file opening have per-platform paths) but untested. On macOS the Ports page
names only your own programs: a port taken by another user's (the system's
Apache, say) is still reported as in use, without a name.

## The stack folder

Everything lives in one folder, laid out like XAMPP's
(default `~/.local/share/slompp`, also on macOS, or `$SLOMPP_ROOT`; MariaDB's
installer needs a path without spaces):

```
htdocs/                      your sites (a welcome page on first run)
apache/conf/httpd.conf       Apache config (yours to edit)
apache/conf/extra/           httpd-ssl.conf, httpd-slompp.conf (PHP, admin tools)
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
`httpd-slompp.conf` predates it appends the `/adminer` alias (restart Apache
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

`slomppctl` is the `lampp` script's counterpart:

```sh
slomppctl start [apache|mysql]
slomppctl stop [apache|mysql]
slomppctl restart [apache|mysql]
slomppctl status
slomppctl ports               # who listens where
slomppctl paths               # where the stack and programs are
slomppctl install-phpmyadmin
slomppctl install-adminer
```

## Layout

- `crates/slompp-core`: no GUI. Finds the programs (`toolchain`), writes the
  stack's config (`setup`, `templates`), starts/stops/watches the servers
  (`stack`, `process`), lists listening ports (`net`), installs phpMyAdmin and Adminer (`dbadmin`),
  and the `slomppctl` binary.
- `crates/slompp`: the GPUI panel and the tray icon.
- `scripts/bundle-macos.sh`: builds `SLOMPP.app`.

Settings are stored in `~/.config/slompp/settings.json` (`$XDG_CONFIG_HOME`
is honoured on macOS as well).

Fonts: IBM Plex Sans and IBM Plex Mono (SIL Open Font License).
