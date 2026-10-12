# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

SLOPP is an XAMPP port: the stock servers (Apache httpd + PHP, MariaDB, phpMyAdmin or Adminer) driven by a Rust control panel on GPUI via `gpui-kit` 0.7.1. Modules are deliberately limited to Apache and MySQL (no Tomcat, FTP or mail). Linux and macOS are supported; keep cross-platform seams (`cfg(unix)`/`cfg(windows)`) intact.

## Commands

On Linux everything runs inside the Nix dev shell (`rustc` is not on the system PATH; GPUI needs the shell's Vulkan/Wayland libraries; the shell also puts Apache, PHP and MariaDB on PATH):

```sh
nix develop -c cargo build -p slopp
nix develop -c cargo run -p slopp
nix develop -c cargo test --workspace
nix develop -c cargo clippy --workspace --all-targets
nix develop -c cargo fmt --all
CARGO_TARGET_DIR=target/core nix develop -c cargo test -p slopp-core    # core only, doesn't wait on the GPUI build lock
nix build                                                               # result/bin/slopp and sloppctl, wrapped with the servers on PATH
```

- The first GPUI build takes ~10 minutes.
- macOS has no Nix shell (the flake is Linux only): use plain `cargo` with Homebrew's `rust`, and `brew install php mariadb` (or keg-only `mariadb@11.8` beside Homebrew's `mysql`; `httpd` is optional, macOS ships one). `scripts/bundle-macos.sh` builds `target/release/SLOPP.app`.
- Releases: bump `version` in the workspace `Cargo.toml`, then push a matching tag (`git tag v0.2.0 && git push origin v0.2.0`). `.github/workflows/release.yml` builds a Linux x86_64 tarball (plain cargo on Ubuntu 24.04, so glibc ≥ 2.39) and an Apple silicon `SLOPP.app` zip, and publishes them as a GitHub release; a tag that doesn't match the version fails the run. "Run workflow" builds without releasing. Nothing else runs CI.
- Try the stack without the GUI: `SLOPP_ROOT=<dir> XDG_CONFIG_HOME=<dir> sloppctl start|stop|status|ports|paths|install-phpmyadmin|install-adminer`.

## Running and inspecting the UI

- Don't drive the GUI with xdotool/ydotool or screenshot the user's desktop. Screenshot in a headless sway instead: `nix shell nixpkgs#sway nixpkgs#grim`, a config with `output * resolution 2200x1500 scale 2` and an `exec` that starts `target/debug/slopp`, sleeps, runs `grim`, then `swaymsg exit`; run sway with `WLR_BACKENDS=headless WLR_RENDERER=gles2 WLR_RENDER_DRM_DEVICE=/dev/dri/renderD128` and `DISPLAY` unset.
- Keep test runs off the user's real stack and settings with `SLOPP_ROOT` and `XDG_CONFIG_HOME`.
- Dev hooks: `SLOPP_PAGE=ports|settings` opens that page, `SLOPP_THEME=light|dark` overrides the system appearance, `SLOPP_HIDDEN=1` starts without a window, `SLOPP_TRAY_ASSUME=1` registers the tray even with no tray host, `SLOPP_LOG=<tracing filter>`. To exercise a start without clicking, put `{"autostart":["mysql"]}` in the test settings.json.
- Test the tray without touching the user's panel: run sway inside `dbus-run-session` with `SLOPP_TRAY_ASSUME=1`, then drive it with `busctl --user`: find `org.kde.StatusNotifierItem-*`, read `ToolTip`/`IconPixmap` on `/StatusNotifierItem`, list the menu with `call -- <name> /MenuBar com.canonical.dbusmenu GetLayout iias 0 -1 0` (the `--` keeps `-1` from being read as an option) and click with `Event isvu <id> clicked s "" 0`. `swaymsg kill` closes the window.
- Only one SLOPP runs per user: a second launch connects to `$XDG_RUNTIME_DIR/slopp-<uid>.sock`, asks the first to show its window and exits. Quit a test instance before starting another.
- Servers started by the panel outlive it by design; stop them with `sloppctl stop` after testing.
- macOS has no headless compositor, so the test instance opens on the real desktop (still with `SLOPP_ROOT`/`XDG_CONFIG_HOME`, and high ports in the test settings.json so it can't collide with a real stack). Capture just its window: find the window number with `CGWindowListCopyWindowInfo` for the pid, then `screencapture -x -o -l<number> out.png`. Drive the menus and the menu bar item through System Events (needs Accessibility permission for the terminal): `osascript -e 'tell application "System Events" to tell process "slopp" to click menu bar item 1 of menu bar 2'` opens the tray menu, then `click menu item "Stop all" of menu 1 of menu bar item 1 of menu bar 2`; `help of menu bar item 1 of menu bar 2` is the tooltip; app menus are on `menu bar 1`. GPUI's own widgets aren't exposed to accessibility.
- In test scripts, never `pkill -f` a pattern that also appears in the script's own command line, and don't start `httpd` as a plain background job of the script's shell: on SIGTERM it signals its whole process group.

## Architecture

- `slopp-core` (no GPUI):
  - `Layout`: every path inside the stack root (XAMPP-like). Unix sockets go to `tmp/` unless the path would exceed the 108-byte socket limit, then to `/tmp/slopp-<uid>-<hash>` (`Layout::sockets`). Never derive it from `$TMPDIR`: the socket path is written into the write-once configs, and `nix develop` changes `$TMPDIR`. The default root and settings live in `~/.local/share` and `~/.config` on macOS too (`layout::base_dir`), not `~/Library/Application Support`: `mariadb-install-db` breaks on the space.
  - `Toolchain::detect`: finds httpd (+ modules dir via `httpd -V`), PHP (prefers `libphp.so` → mod_php + prefork, else PHP-FPM + event) and MariaDB on PATH plus sbin dirs; `Settings.binaries` overrides. On macOS `search_dirs` adds Homebrew/MacPorts prefixes and unlinked Homebrew kegs (`opt/{httpd,php,mariadb}[@version]`) and puts `/usr/sbin` last, so a Dock launch (bare PATH) finds the same programs as a terminal; Apple's `/usr/sbin/httpd` refuses unsigned modules, so it always gets PHP-FPM.
  - `setup`: creates dirs; user-facing config files are **write-once** (`write_once`), never regenerated. `php-fpm.conf` in `run/` is internal and rewritten each start.
  - Machine-specific values reach Apache as `-C "Define …"`/`-D` flags (`SLOPP_MODULES`, `HTTP_PORT`, `HTTPS_PORT`, `SSL`, `PHP_FPM`/`PHP_FPM_SOCKET`, `PHP_MODULE`/`PHP_MODULE_PATH`) built in `Stack::start_apache`; MariaDB gets `--port`. Keep it that way so ports/paths never require rewriting user configs.
  - `Stack::start` runs `httpd -t` first so config errors surface with line numbers, then spawns servers in the foreground (`-DFOREGROUND`, `php-fpm --nodaemonize`, `mariadbd`) detached into their own process group (`process::spawn_detached`, which also clears the inherited signal mask and dispositions: GPUI's background threads block SIGTERM on macOS, and a server that inherits that can't be stopped). It records `pid starttime` lines in `run/<module>.pids`; the start time catches reused pids (`/proc/<pid>/stat` on Linux, `proc_pidinfo` on macOS). `Stack` keeps `Child` handles and reaps them in `status()` so no zombies pile up while the panel runs.
  - `net`: `port_open` (connect probe) and `listeners()` (Linux: parses `/proc/net/tcp{,6}` and maps socket inodes to pids; macOS: `lsof -F`, own processes only. `netstat` prints no TCP sockets when its caller isn't an Apple binary).
  - `desktop`: on macOS an editor/terminal setting that isn't a program is an application name for `open -a`; the Shell button hands the terminal app a generated `run/shell.command`, since `open` passes neither environment nor working directory.
  - `dbadmin::DbAdmin` (phpMyAdmin, Adminer): the admin tools, downloaded on request and served at `/phpmyadmin/` and `/adminer/`; `Stack::db_admin_url` once installed. `phpmyadmin` downloads the latest release (version.txt → files.phpmyadmin.net zip) plus the pinned, SHA-256-checked `boodark-orange` theme (BooDark 1.2.0's own "orange" is cyan relabelled), keeps an existing `config.inc.php` and adds `ThemeDefault` to it if missing. `adminer` reads the version from adminer.org's `latest-mysql.php` redirect, replaces `adminer/adminer.php` and writes `adminer/index.php` and `adminer/adminer.css` (the panel's palette, light/dark by `prefers-color-scheme`, overriding Adminer's `default.css` and `dark.css`) once. `index.php` subclasses `Adminer\Adminer` (socket credentials, `login()` true, `verifyLoginToken()` false) and posts a permanent login itself whenever there's no `username` or no Adminer cookie, since Adminer refuses password-less logins. Older stacks' `httpd-slopp.conf` gets the `/adminer` alias appended (`Installed::apache_conf_changed`).
- `slopp` (GPUI):
  - `controller::Controller` is an app-level model (reach it with `Controller::global(cx)`), independent of any window: owns `Arc<Stack>`, the log, a poll loop (1.5 s, 0.4 s while busy) that computes status/problems on the background executor, and per-module `Phase` (Idle/Starting{launched}/Stopping/Failed) layered over OS status. Transitions are logged in `apply_snapshot`. `changed()` notifies views and pushes a `TrayState` to the tray when it differs. Saving settings replaces the stack via `Stack::reconfigure` (children move over). Window-only feedback goes out as `ControllerEvent::Notify`.
  - `panel::Panel` is the window: page, console scroll and the settings form, rendering the controller. View helpers take `(&Controller, &Context<Panel>)`; `panel::on_controller(cx, |c, cx| …)` makes click handlers that act on the controller.
  - `tray.rs`: the menu (`TrayState::menu`) and the icon, rasterised in code (`tray::icon`: the mark, three units joined into an S by grey links, top lamp Apache, middle MySQL; white or black units by `tray::light_panel`, a lit lamp a gradient of its colour over the unit's right side, in shades per ink; `Ink::Template` on macOS while no lamp is lit). `brand::mark` draws the same mark with live lamps in the title bar, from the same colours (`theme::MarkInk`); `assets/linux/org.slopp.Slopp.svg` (also the macOS icon) is the mark with no lamps lit, are shared; Linux shows them with `ksni` on the async-io runtime like GPUI's zbus, macOS (`tray/macos.rs`) as an `NSStatusItem` through `objc2-app-kit` on the main thread. menu clicks and second launches (`instance.rs`) arrive in `main.rs` as `TrayCommand`s over one channel. `main.rs` tracks the window (`PanelWindow`) and quits on window close unless a tray exists and `close_to_tray` is set; GPUI's quit mode is `Explicit` so the app can outlive its window. `menus.rs` (macOS only) sets the menu bar menus and Cmd-W/M/H; other shortcuts are bound as `secondary-…` (Cmd on macOS, Ctrl elsewhere).
  - `views/rack.rs` (module units: status rail + lamp, ports, PIDs, actions; MySQL's Admin is a menu to open or get each `DbAdmin`), `console.rs` (log), `ports.rs`, `settings.rs` (`SettingsView` emits `SettingsEvent::Save`), `brand.rs` (title bar).
  - `theme.rs`: palette tokens for dark/light (`Palette`), bundled IBM Plex Sans/Mono, gpui-component restyle. Colour comes from status lamps; XAMPP orange (`accent`) only for the brand and the Start action. Use tokens, not hard-coded colours.

## GPUI gotchas

- `overflow_hidden` doesn't clip to rounded corners: the rack's rails round their own outer corners.
- Wrap clickable things inside `TitleBar` with `brand::controls()` or KWin's window move swallows the click.
- An app-level action handler that needs the window (`menus::in_active_window`) must `cx.defer`: the action is dispatched from inside that window's update.
- `ScrollHandle::scroll_to_bottom` applies at the next layout, so call it from render (the console follows `Log::revision`).
