# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

SLOMPP is an XAMPP port: the stock servers (Apache httpd + PHP, MariaDB, phpMyAdmin) driven by a Rust control panel on GPUI via `gpui-kit` 0.7.1. Modules are deliberately limited to Apache and MySQL (no Tomcat, FTP or mail). Linux first; keep cross-platform seams (`cfg(unix)`/`cfg(windows)`) intact.

## Commands

Everything runs inside the Nix dev shell (`rustc` is not on the system PATH; GPUI needs the shell's Vulkan/Wayland libraries; the shell also puts Apache, PHP and MariaDB on PATH):

```sh
nix develop -c cargo build -p slompp
nix develop -c cargo run -p slompp
nix develop -c cargo test --workspace
nix develop -c cargo clippy --workspace --all-targets
nix develop -c cargo fmt --all
CARGO_TARGET_DIR=target/core nix develop -c cargo test -p slompp-core   # core only, doesn't wait on the GPUI build lock
nix build                                                               # result/bin/slompp and slomppctl, wrapped with the servers on PATH
```

- The first GPUI build takes ~10 minutes.
- Try the stack without the GUI: `SLOMPP_ROOT=<dir> XDG_CONFIG_HOME=<dir> slomppctl start|stop|status|ports|paths|install-phpmyadmin`.

## Running and inspecting the UI

- Don't drive the GUI with xdotool/ydotool or screenshot the user's desktop. Screenshot in a headless sway instead: `nix shell nixpkgs#sway nixpkgs#grim`, a config with `output * resolution 2200x1500 scale 2` and an `exec` that starts `target/debug/slompp`, sleeps, runs `grim`, then `swaymsg exit`; run sway with `WLR_BACKENDS=headless WLR_RENDERER=gles2 WLR_RENDER_DRM_DEVICE=/dev/dri/renderD128` and `DISPLAY` unset.
- Keep test runs off the user's real stack and settings with `SLOMPP_ROOT` and `XDG_CONFIG_HOME`.
- Dev hooks: `SLOMPP_PAGE=ports|settings` opens that page, `SLOMPP_THEME=light|dark` overrides the system appearance, `SLOMPP_HIDDEN=1` starts without a window, `SLOMPP_TRAY_ASSUME=1` registers the tray even with no tray host, `SLOMPP_LOG=<tracing filter>`. To exercise a start without clicking, put `{"autostart":["mysql"]}` in the test settings.json.
- Test the tray without touching the user's panel: run sway inside `dbus-run-session` with `SLOMPP_TRAY_ASSUME=1`, then drive it with `busctl --user`: find `org.kde.StatusNotifierItem-*`, read `ToolTip`/`IconPixmap` on `/StatusNotifierItem`, list the menu with `call -- <name> /MenuBar com.canonical.dbusmenu GetLayout iias 0 -1 0` (the `--` keeps `-1` from being read as an option) and click with `Event isvu <id> clicked s "" 0`. `swaymsg kill` closes the window.
- Only one SLOMPP runs per user: a second launch connects to `$XDG_RUNTIME_DIR/slompp-<uid>.sock`, asks the first to show its window and exits. Quit a test instance before starting another.
- Servers started by the panel outlive it by design; stop them with `slomppctl stop` after testing.

## Architecture

- `slompp-core` (no GPUI):
  - `Layout`: every path inside the stack root (XAMPP-like). Unix sockets go to `tmp/` unless the path would exceed the 108-byte socket limit, then to `/tmp/slompp-<uid>-<hash>` (`Layout::sockets`). Never derive it from `$TMPDIR`: the socket path is written into the write-once configs, and `nix develop` changes `$TMPDIR`.
  - `Toolchain::detect`: finds httpd (+ modules dir via `httpd -V`), PHP (prefers `libphp.so` → mod_php + prefork, else PHP-FPM + event) and MariaDB on PATH plus sbin dirs; `Settings.binaries` overrides.
  - `setup`: creates dirs; user-facing config files are **write-once** (`write_once`), never regenerated. `php-fpm.conf` in `run/` is internal and rewritten each start.
  - Machine-specific values reach Apache as `-C "Define …"`/`-D` flags (`SLOMPP_MODULES`, `HTTP_PORT`, `HTTPS_PORT`, `SSL`, `PHP_FPM`/`PHP_FPM_SOCKET`, `PHP_MODULE`/`PHP_MODULE_PATH`) built in `Stack::start_apache`; MariaDB gets `--port`. Keep it that way so ports/paths never require rewriting user configs.
  - `Stack::start` runs `httpd -t` first so config errors surface with line numbers, then spawns servers in the foreground (`-DFOREGROUND`, `php-fpm --nodaemonize`, `mariadbd`) detached into their own process group (`process::spawn_detached`). It records `pid starttime` lines in `run/<module>.pids`; the start time catches reused pids. `Stack` keeps `Child` handles and reaps them in `status()` so no zombies pile up while the panel runs.
  - `net`: `port_open` (connect probe) and `listeners()` (parses `/proc/net/tcp{,6}` and maps socket inodes to pids; Linux only).
  - `phpmyadmin`: downloads the latest release (version.txt → files.phpmyadmin.net zip), keeps an existing `config.inc.php`.
- `slompp` (GPUI):
  - `controller::Controller` is an app-level model (reach it with `Controller::global(cx)`), independent of any window: owns `Arc<Stack>`, the log, a poll loop (1.5 s, 0.4 s while busy) that computes status/problems on the background executor, and per-module `Phase` (Idle/Starting{launched}/Stopping/Failed) layered over OS status. Transitions are logged in `apply_snapshot`. `changed()` notifies views and pushes a `TrayState` to the tray when it differs. Saving settings replaces the stack via `Stack::reconfigure` (children move over). Window-only feedback goes out as `ControllerEvent::Notify`.
  - `panel::Panel` is the window: page, console scroll and the settings form, rendering the controller. View helpers take `(&Controller, &Context<Panel>)`; `panel::on_controller(cx, |c, cx| …)` makes click handlers that act on the controller.
  - `tray.rs` (Linux only, `ksni` on the async-io runtime like GPUI's zbus): the icon is rasterised in code (`tray::icon`, top lamp Apache, middle MySQL); menu clicks and second launches (`instance.rs`) arrive in `main.rs` as `TrayCommand`s over one channel. `main.rs` tracks the window (`PanelWindow`) and quits on window close unless a tray exists and `close_to_tray` is set; GPUI's quit mode is `Explicit` so the app can outlive its window.
  - `views/rack.rs` (module units: status rail + lamp, ports, PIDs, actions), `console.rs` (log), `ports.rs`, `settings.rs` (`SettingsView` emits `SettingsEvent::Save`), `brand.rs` (title bar).
  - `theme.rs`: palette tokens for dark/light (`Palette`), bundled IBM Plex Sans/Mono, gpui-component restyle. Colour comes from status lamps; XAMPP orange (`accent`) only for the brand and the Start action. Use tokens, not hard-coded colours.

## GPUI gotchas

- `overflow_hidden` doesn't clip to rounded corners: the rack's rails round their own outer corners.
- Wrap clickable things inside `TitleBar` with `brand::controls()` or KWin's window move swallows the click.
- `ScrollHandle::scroll_to_bottom` applies at the next layout, so call it from render (the console follows `Log::revision`).
