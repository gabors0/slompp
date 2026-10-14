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
in the menu bar, on Windows in the notification area (Windows 11 first puts
new icons behind the ^ arrow; drag it onto the taskbar to keep it in view).
Its top lamp is Apache, the middle one MySQL. The menu starts and stops
modules and opens the site, phpMyAdmin or Adminer; on Linux and Windows a
left click shows the panel (the menu is on the right button on Windows).

With a tray, closing the window keeps SLOPP running there (Settings →
Startup turns that off); without one, closing the window quits. Launching
SLOPP again while it runs brings its window back instead of starting a
second copy, and so does a click on its Dock icon on macOS.
`SLOPP_HIDDEN=1 slopp` starts it in the tray only, e.g. for login autostart.

## Downloads

[Releases](https://github.com/gabors0/slopp/releases) have builds for Linux
(x86_64, glibc 2.39 or newer), macOS (Apple silicon) and Windows (x86_64,
Windows 10 or newer). Install the servers first, as below; SLOPP doesn't
bundle them.

- **Linux:** make the AppImage executable (`chmod +x`) and run it. On launch
  it adds itself to your applications (a desktop file and an icon in
  `~/.local/share/applications` and `~/.local/share/icons`), which Wayland
  desktops need to show its taskbar icon. It updates them if you move the
  AppImage and leaves them to AppImageLauncher, Gear Lever or an installed
  package if one of those has SLOPP already. To remove them, delete
  `org.slopp.Slopp.desktop` and `org.slopp.Slopp.svg` there;
  `~/.local/share/appimagekit/no_desktopintegration` (an empty file) stops
  AppImages adding themselves. A symlink to the AppImage named `sloppctl`
  runs `sloppctl`. The tarball has the plain binaries instead. Either needs
  libxkbcommon, Wayland or X11, and a Vulkan driver, which desktop installs
  usually have.
- **NixOS:** the release binaries won't find their libraries; use the flake
  instead, which also puts Apache, PHP and MariaDB on `PATH`:
  `nix run github:gabors0/slopp`, or
  `nix profile install github:gabors0/slopp` to keep it. The AppImage runs
  with `appimage-run`, and its applications entry then starts it that way.
- **macOS:** open the dmg and drag `SLOPP.app` to Applications. The app
  isn't notarized, so macOS blocks the first launch: allow it in System
  Settings → Privacy & Security (Open Anyway), or run
  `xattr -dr com.apple.quarantine /Applications/SLOPP.app`.
- **Windows:** the setup installs SLOPP for your user without admin rights
  (or for everyone, if you choose) with a Start menu entry; uninstalling it
  stops the servers but keeps the stack folder and settings. The zip is the
  portable alternative: unzip and run `slopp.exe`, or `sloppctl.exe` from a
  terminal. Neither is signed, so SmartScreen may warn the first time: More
  info → Run anyway.

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

On Windows (new, so expect rough edges), install the 64-bit builds:

- Apache from [Apache Lounge](https://www.apachelounge.com/download/),
  unpacked to `C:\Apache24` as its readme says.
- PHP's **Thread Safe** zip from [windows.php.net](https://windows.php.net/download/),
  unpacked to `C:\php`. Only that build has the Apache module
  (`php8apache2_4.dll`); Windows has no PHP-FPM.
- MariaDB from its installer on [mariadb.org](https://mariadb.org/download/).
  Don't let it set up a Windows service on port 3306, or choose another port
  in SLOPP.

SLOPP also looks on `PATH`, in Chocolatey's and Scoop's folders,
`C:\Program Files\MariaDB *` and `C:\xampp`, so an existing XAMPP's Apache,
PHP and MariaDB work too. Settings → Programs takes explicit paths. PHP loads
the extensions phpMyAdmin and most frameworks need (`extension=` lines in the
stack's `php.ini`); SLOPP puts PHP's folder on Apache's `PATH` and points
`extension_dir` at its `ext` folder when it starts Apache. phpMyAdmin and
Adminer reach MariaDB over TCP on localhost (MariaDB for Windows has named
pipes, off by default, instead of a Unix socket).

Linux, macOS and Windows are supported. On macOS the Ports page
names only your own programs: a port taken by another user's (the system's
Apache, say) is still reported as in use, without a name.

## The stack folder

Everything lives in one folder, laid out like XAMPP's
(default `~/.local/share/slopp`, also on macOS, `%LOCALAPPDATA%\slopp` on
Windows, or `$SLOPP_ROOT`; on Linux and macOS MariaDB's installer needs a
path without spaces):

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
- `scripts/bundle-macos.sh`: builds `SLOPP.app` (and with `--dmg` the disk
  image), `scripts/bundle-linux.sh` the AppImage, and
  `crates/slopp/assets/windows/slopp.iss` is the Windows installer (Inno
  Setup). `scripts/icons.sh` renders the Windows and X11 icons from the SVG.

Settings are stored in `~/.config/slopp/settings.json` (`$XDG_CONFIG_HOME`
is honoured on macOS as well), on Windows in `%APPDATA%\slopp\settings.json`.

Fonts: IBM Plex Sans and IBM Plex Mono (SIL Open Font License).

## License

SLOPP is licensed under the [GNU General Public License v3.0 or later](LICENSE). Apache httpd, PHP,
MariaDB, phpMyAdmin and Adminer are separate programs with their own licenses;
SLOPP runs them as child processes and doesn't include or link them.
