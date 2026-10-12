# SLOMPP

An XAMPP-style local web stack with a control panel written in Rust on
[GPUI](https://gpui-kit.com). The servers are the stock upstream programs, the
same modules XAMPP ships:

| Module  | What runs                                        |
| ------- | ------------------------------------------------ |
| Apache  | Apache httpd, with PHP through PHP-FPM or mod_php |
| MySQL   | MariaDB                                          |
| (admin) | phpMyAdmin, downloaded on request                |

Tomcat, ProFTPD/FileZilla and Mercury are not included.

SLOMPP manages the servers the way the XAMPP control panel does: start and
stop each module, see its ports and PIDs, open its config and log files, open
the site or phpMyAdmin, and spot port conflicts before they bite. The servers
keep running when the panel closes (unless you change that in Settings).

## Running it

On NixOS / with Nix:

```sh
nix run                      # the panel, with Apache, PHP and MariaDB on PATH
nix develop -c cargo run -p slompp
```

Elsewhere, install Apache, PHP-FPM (or PHP's Apache module) and the MariaDB
server with your package manager, then `cargo run -p slompp`. SLOMPP looks
for them on `PATH` and in the usual system locations; Settings → Programs
takes explicit paths.

Linux is the supported platform for now. The code keeps macOS and Windows in
mind (process handling and file opening have per-platform paths), but they
are untested and the Ports page lists sockets on Linux only.

## The stack folder

Everything lives in one folder, laid out like XAMPP's
(default `~/.local/share/slompp`, or `$SLOMPP_ROOT`):

```
htdocs/                      your sites (a welcome page on first run)
apache/conf/httpd.conf       Apache config (yours to edit)
apache/conf/extra/           httpd-ssl.conf, httpd-slompp.conf (PHP, phpMyAdmin)
apache/logs/                 error.log, access.log
php/php.ini                  PHP config, php/logs/php_error_log
mysql/my.cnf                 MariaDB config, mysql/data/ the databases
phpMyAdmin/                  after "Get phpMyAdmin"
tmp/  run/                   sockets, sessions, pid files
```

Config files are written once and never overwritten. Ports and
machine-specific paths (Apache's module directory, PHP-FPM's socket) are
passed on the command line at start, so changing ports in Settings doesn't
touch your edits.

Like XAMPP, MariaDB's `root` user has no password and phpMyAdmin logs in as
root. Both only listen on localhost; don't expose this stack to a network.

Ports default to XAMPP's 80/443/3306 where the user may bind them, otherwise
8080/8443/3306.

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
```

## Layout

- `crates/slompp-core`: no GUI. Finds the programs (`toolchain`), writes the
  stack's config (`setup`, `templates`), starts/stops/watches the servers
  (`stack`, `process`), lists listening ports (`net`), installs phpMyAdmin,
  and the `slomppctl` binary.
- `crates/slompp`: the GPUI panel.

Settings are stored in `~/.config/slompp/settings.json`.

Fonts: IBM Plex Sans and IBM Plex Mono (SIL Open Font License).
