//! The control panel: owns the stack, watches the modules, runs what the
//! buttons ask for, and keeps the log.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::notification::Notification;
use gpui_kit::component::{TitleBar, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, KeyBinding, ScrollHandle, Subscription, Task,
    Window, WindowAppearance, actions, div,
};
use slompp_core::net::Listener;
use slompp_core::phpmyadmin::{self, Progress};
use slompp_core::{Module, ModuleStatus, Problem, Settings, Stack, desktop};

use crate::log::{Kind, Log};
use crate::theme::{self, Lamp, Palette};
use crate::views::settings::{SettingsEvent, SettingsView};
use crate::views::{brand, console, ports, rack};

actions!(
    slompp,
    [
        Quit,
        StartAll,
        StopAll,
        ShowModules,
        ShowPorts,
        ShowSettings
    ]
);

const CONTEXT: &str = "Panel";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("ctrl-1", ShowModules, Some(CONTEXT)),
        KeyBinding::new("ctrl-2", ShowPorts, Some(CONTEXT)),
        KeyBinding::new("ctrl-3", ShowSettings, Some(CONTEXT)),
        KeyBinding::new("ctrl-,", ShowSettings, Some(CONTEXT)),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Modules,
    Ports,
    Settings,
}

/// What the panel is doing with a module, on top of what the OS reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    /// `launched` is when the processes were started; `None` while setup
    /// (config, first-run database creation) is still running.
    Starting {
        launched: Option<Instant>,
    },
    Stopping,
    /// The last start failed or the module died; the log says why.
    Failed,
}

pub struct Row {
    pub module: Module,
    pub status: Option<ModuleStatus>,
    pub problems: Vec<Problem>,
    pub phase: Phase,
}

impl Row {
    pub fn running(&self) -> bool {
        self.status.as_ref().is_some_and(ModuleStatus::running)
    }

    pub fn busy(&self) -> bool {
        matches!(self.phase, Phase::Starting { .. } | Phase::Stopping)
    }

    pub fn lamp(&self) -> Lamp {
        match self.phase {
            Phase::Starting { .. } | Phase::Stopping => Lamp::Busy,
            Phase::Failed => Lamp::Fault,
            Phase::Idle => match &self.status {
                Some(s) if s.ready() => Lamp::On,
                Some(s) if s.running() => Lamp::Busy,
                _ => Lamp::Off,
            },
        }
    }

    pub fn state_label(&self) -> &'static str {
        match self.phase {
            Phase::Starting { .. } => "Starting…",
            Phase::Stopping => "Stopping…",
            Phase::Failed if !self.running() => "Stopped with an error",
            _ => match &self.status {
                Some(s) if s.ready() => "Running",
                Some(s) if s.running() => "Opening ports…",
                _ => "Stopped",
            },
        }
    }
}

/// phpMyAdmin's installer, shown in the MySQL row.
#[derive(Debug, Clone, PartialEq)]
pub enum PmaState {
    Idle,
    Installing(String),
}

pub struct Panel {
    stack: Arc<Stack>,
    settings_path: PathBuf,
    pub page: Page,
    pub rows: Vec<Row>,
    pub log: Log,
    pub log_scroll: ScrollHandle,
    /// The log revision the console last scrolled down to.
    scrolled_revision: usize,
    pub listeners: Vec<Listener>,
    pub pma: PmaState,
    settings_view: Entity<SettingsView>,
    focus: FocusHandle,
    _poll: Task<()>,
    _subscriptions: Vec<Subscription>,
}

/// One poll: each module's status and problems, and the listening ports
/// when the Ports page is open.
type Snapshot = (Vec<(ModuleStatus, Vec<Problem>)>, Option<Vec<Listener>>);

/// How long a launched module gets to open its ports.
const START_TIMEOUT: Duration = Duration::from_secs(45);

impl Panel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings_path = Settings::default_path();
        let mut log = Log::default();
        let settings = match Settings::load(&settings_path) {
            Ok(s) => s,
            Err(err) => {
                log.push(
                    None,
                    Kind::Error,
                    format!("Using default settings: {err:#}"),
                );
                Settings::default()
            }
        };
        let stack = Arc::new(Stack::new(settings));

        let settings_view = cx.new(|cx| SettingsView::new(&stack, window, cx));
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        let subscriptions = vec![
            cx.subscribe_in(&settings_view, window, Self::on_settings_event),
            cx.observe_window_appearance(window, |_, window, cx| {
                theme::apply(wants_light(window.appearance()), cx);
                cx.notify();
            }),
            cx.on_app_quit(|this, _| {
                let stack = this.stack.clone();
                let stop = stack.settings().stop_on_exit;
                async move {
                    if stop {
                        for m in Module::ALL {
                            let _ = stack.stop(m);
                        }
                    }
                }
            }),
        ];

        let rows = Module::ALL
            .into_iter()
            .map(|module| Row {
                module,
                status: None,
                problems: Vec::new(),
                phase: Phase::Idle,
            })
            .collect();

        let page = match std::env::var("SLOMPP_PAGE").as_deref() {
            Ok("ports") => Page::Ports,
            Ok("settings") => Page::Settings,
            _ => Page::Modules,
        };

        let mut this = Self {
            stack,
            settings_path,
            page,
            rows,
            log,
            log_scroll: ScrollHandle::new(),
            scrolled_revision: usize::MAX,
            listeners: Vec::new(),
            pma: PmaState::Idle,
            settings_view,
            focus,
            _poll: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.greet();
        this._poll = this.poll_loop(cx);
        this.autostart(cx);
        this
    }

    pub fn stack(&self) -> &Arc<Stack> {
        &self.stack
    }

    pub fn row(&self, module: Module) -> &Row {
        self.rows.iter().find(|r| r.module == module).unwrap()
    }

    fn row_mut(&mut self, module: Module) -> &mut Row {
        self.rows.iter_mut().find(|r| r.module == module).unwrap()
    }

    /// The opening lines: where the stack is and what runs it.
    fn greet(&mut self) {
        let root = self.stack.layout().root().display().to_string();
        self.log.push(
            None,
            Kind::Info,
            format!("SLOMPP {}", env!("CARGO_PKG_VERSION")),
        );
        self.log
            .push(None, Kind::Info, format!("Stack root: {root}"));
        for m in Module::ALL {
            match self.stack.describe(m) {
                Some(text) => self.log.push(Some(m), Kind::Info, text),
                None => {
                    for problem in self.stack.problems(m) {
                        if let Problem::Missing { .. } = problem {
                            self.log.push(Some(m), Kind::Error, problem.to_string());
                        }
                    }
                }
            }
        }
    }

    fn autostart(&mut self, cx: &mut Context<Self>) {
        let modules = self.stack.settings().autostart.clone();
        if modules.is_empty() {
            return;
        }
        // Wait for the first poll so modules already running are left alone.
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(700))
                .await;
            this.update(cx, |this, cx| {
                for m in modules {
                    if !this.row(m).running() {
                        this.log.push(Some(m), Kind::Info, "Autostart");
                        this.start(m, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn poll_loop(&self, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut first = true;
            loop {
                let Ok((stack, busy, want_ports)) = this.read_with(cx, |this, _| {
                    (
                        this.stack.clone(),
                        this.rows.iter().any(Row::busy),
                        this.page == Page::Ports,
                    )
                }) else {
                    return;
                };
                if !first {
                    let wait = if busy { 400 } else { 1500 };
                    cx.background_executor()
                        .timer(Duration::from_millis(wait))
                        .await;
                }
                first = false;
                let snapshot = cx
                    .background_spawn(async move {
                        let modules: Vec<_> = Module::ALL
                            .into_iter()
                            .map(|m| {
                                let status = stack.status(m);
                                let problems = if status.running() {
                                    Vec::new()
                                } else {
                                    stack.problems(m)
                                };
                                (status, problems)
                            })
                            .collect();
                        let listeners = want_ports.then(slompp_core::net::listeners);
                        (modules, listeners)
                    })
                    .await;
                if this
                    .update(cx, |this, cx| this.apply_snapshot(snapshot, cx))
                    .is_err()
                {
                    return;
                }
            }
        })
    }

    fn apply_snapshot(&mut self, (modules, listeners): Snapshot, cx: &mut Context<Self>) {
        for (status, problems) in modules {
            let m = status.module;
            let was_running = self.row(m).status.as_ref().map(ModuleStatus::running);
            let phase = self.row(m).phase;
            match phase {
                Phase::Starting {
                    launched: Some(launched),
                } => {
                    if status.ready() {
                        self.row_mut(m).phase = Phase::Idle;
                        let ports: Vec<String> =
                            status.ports.iter().map(|p| p.port.to_string()).collect();
                        let what = if ports.len() == 1 { "port" } else { "ports" };
                        self.log.push(
                            Some(m),
                            Kind::Ok,
                            format!("Running on {what} {}", ports.join(" and ")),
                        );
                    } else if !status.running() {
                        self.row_mut(m).phase = Phase::Failed;
                        let details = self.stack.failure_details(m);
                        self.log.push(
                            Some(m),
                            Kind::Error,
                            format!("{m} stopped while starting.\n{details}"),
                        );
                    } else if launched.elapsed() > START_TIMEOUT {
                        self.row_mut(m).phase = Phase::Failed;
                        self.log.push(
                            Some(m),
                            Kind::Error,
                            format!(
                                "{m} is running but did not open its ports within {}s.",
                                START_TIMEOUT.as_secs()
                            ),
                        );
                    }
                }
                Phase::Starting { launched: None } | Phase::Stopping => {}
                Phase::Idle | Phase::Failed => {
                    if was_running == Some(true) && !status.running() {
                        self.row_mut(m).phase = Phase::Failed;
                        let details = self.stack.failure_details(m);
                        self.log.push(
                            Some(m),
                            Kind::Error,
                            format!("{m} stopped unexpectedly.\n{details}"),
                        );
                    } else if was_running == Some(false) && status.running() {
                        self.row_mut(m).phase = Phase::Idle;
                        self.log.push(
                            Some(m),
                            Kind::Info,
                            format!("{m} was started outside the panel"),
                        );
                    } else if was_running.is_none() && status.running() {
                        self.log
                            .push(Some(m), Kind::Info, format!("{m} is already running"));
                    }
                }
            }
            // Report a port conflict once, when it appears.
            let row = self.row_mut(m);
            let new_conflicts: Vec<String> = problems
                .iter()
                .filter(|p| matches!(p, Problem::PortInUse { .. }) && !row.problems.contains(p))
                .map(Problem::to_string)
                .collect();
            row.status = Some(status);
            row.problems = problems;
            for text in new_conflicts {
                self.log.push(Some(m), Kind::Error, text);
            }
        }
        if let Some(listeners) = listeners {
            self.listeners = listeners;
        }
        cx.notify();
    }

    pub fn toggle(&mut self, module: Module, cx: &mut Context<Self>) {
        if self.row(module).running() {
            self.stop(module, cx);
        } else {
            self.start(module, cx);
        }
    }

    pub fn start(&mut self, module: Module, cx: &mut Context<Self>) {
        if self.row(module).busy() {
            return;
        }
        self.row_mut(module).phase = Phase::Starting { launched: None };
        let first_db =
            module == Module::MySql && !slompp_core::setup::mysql_initialised(self.stack.layout());
        self.log.push(
            Some(module),
            Kind::Info,
            if first_db {
                "Creating the database (first start)…".to_string()
            } else {
                format!("Starting {module}…")
            },
        );
        let stack = self.stack.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { stack.start(module) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(pids) => {
                        this.row_mut(module).phase = Phase::Starting {
                            launched: Some(Instant::now()),
                        };
                        let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
                        let what = if pids.len() == 1 { "PID" } else { "PIDs" };
                        this.log.push(
                            Some(module),
                            Kind::Info,
                            format!("Launched ({what} {})", pids.join(", ")),
                        );
                    }
                    Err(err) => {
                        this.row_mut(module).phase = Phase::Failed;
                        this.log.push(
                            Some(module),
                            Kind::Error,
                            format!("{module} didn't start: {err:#}"),
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn stop(&mut self, module: Module, cx: &mut Context<Self>) {
        if self.row(module).busy() {
            return;
        }
        self.row_mut(module).phase = Phase::Stopping;
        self.log
            .push(Some(module), Kind::Info, format!("Stopping {module}…"));
        let stack = self.stack.clone();
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { stack.stop(module) }).await;
            this.update(cx, |this, cx| {
                // The next poll shouldn't read this as an unexpected exit.
                if let Some(status) = &mut this.row_mut(module).status {
                    status.pids.clear();
                }
                match result {
                    Ok(killed) => {
                        this.row_mut(module).phase = Phase::Idle;
                        this.log.push(
                            Some(module),
                            Kind::Ok,
                            if killed {
                                "Stopped (it didn't exit in time and was killed)"
                            } else {
                                "Stopped"
                            },
                        );
                    }
                    Err(err) => {
                        this.row_mut(module).phase = Phase::Failed;
                        this.log.push(
                            Some(module),
                            Kind::Error,
                            format!("Stopping {module} failed: {err:#}"),
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn start_all(&mut self, _: &StartAll, _: &mut Window, cx: &mut Context<Self>) {
        for m in Module::ALL {
            if !self.row(m).running() {
                self.start(m, cx);
            }
        }
    }

    pub fn stop_all(&mut self, _: &StopAll, _: &mut Window, cx: &mut Context<Self>) {
        for m in Module::ALL {
            if self.row(m).running() {
                self.stop(m, cx);
            }
        }
    }

    pub fn any_running(&self) -> bool {
        self.rows.iter().any(Row::running)
    }

    pub fn all_running(&self) -> bool {
        self.rows.iter().all(Row::running)
    }

    pub fn show(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        if page == Page::Ports {
            self.refresh_ports(cx);
        }
        cx.notify();
    }

    pub fn refresh_ports(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let listeners = cx
                .background_spawn(async { slompp_core::net::listeners() })
                .await;
            this.update(cx, |this, cx| {
                this.listeners = listeners;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Opens a config or log file in the editor from Settings.
    pub fn open_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let editor = self.stack.settings().editor.clone();
        if let Err(err) = desktop::open_path(&path, editor.as_deref()) {
            self.log.push(None, Kind::Error, format!("{err:#}"));
            cx.notify();
        }
    }

    pub fn open_admin(&mut self, module: Module, cx: &mut Context<Self>) {
        let Some(url) = self.stack.admin_url(module) else {
            return;
        };
        if !self.row(Module::Apache).running() {
            self.log.push(
                Some(module),
                Kind::Info,
                "Start Apache first: the admin pages are served by it.",
            );
            cx.notify();
            return;
        }
        if let Err(err) = desktop::open_url(&url) {
            self.log.push(None, Kind::Error, format!("{err:#}"));
            cx.notify();
        }
    }

    pub fn open_root(&mut self, cx: &mut Context<Self>) {
        let root = self.stack.layout().root().to_path_buf();
        self.open_folder(root, cx);
    }

    /// Opens a folder in the file manager, creating it if needed.
    pub fn open_folder(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        if let Err(err) = std::fs::create_dir_all(&dir)
            .map_err(anyhow::Error::from)
            .and_then(|_| desktop::open_path(&dir, None))
        {
            self.log.push(None, Kind::Error, format!("{err:#}"));
            cx.notify();
        }
    }

    pub fn open_shell(&mut self, cx: &mut Context<Self>) {
        let root = self.stack.layout().root().to_path_buf();
        let _ = std::fs::create_dir_all(&root);
        let env = self.stack.shell_env();
        let terminal = self.stack.settings().terminal.clone();
        match desktop::open_terminal(&root, &env, terminal.as_deref()) {
            Ok(()) => self.log.push(
                None,
                Kind::Info,
                "Opened a shell with the stack's programs on PATH",
            ),
            Err(err) => self.log.push(None, Kind::Error, format!("{err:#}")),
        }
        cx.notify();
    }

    pub fn install_phpmyadmin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.pma, PmaState::Installing(_)) {
            return;
        }
        self.pma = PmaState::Installing("Looking up the latest version…".into());
        self.log
            .push(Some(Module::MySql), Kind::Info, "Installing phpMyAdmin…");
        let layout = self.stack.layout().clone();
        let port = self.stack.settings().ports.mysql;
        let (tx, rx) = async_channel::unbounded::<Result<Progress, Result<String, String>>>();
        std::thread::spawn(move || {
            let result = phpmyadmin::install(&layout, port, |p| {
                let _ = tx.send_blocking(Ok(p));
            });
            let _ = tx.send_blocking(Err(result.map_err(|e| format!("{e:#}"))));
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            while let Ok(message) = rx.recv().await {
                let done = message.is_err();
                let ok = this.update(cx, |this, cx| {
                    match message {
                        Ok(progress) => this.pma = PmaState::Installing(progress_text(&progress)),
                        Err(result) => {
                            this.pma = PmaState::Idle;
                            match result {
                                Ok(version) => {
                                    let text = format!("phpMyAdmin {version} is installed");
                                    this.log.push(Some(Module::MySql), Kind::Ok, text.clone());
                                    let _ = window_handle.update(cx, |_, window, cx| {
                                        window.push_notification(Notification::success(text), cx)
                                    });
                                }
                                Err(err) => this.log.push(
                                    Some(Module::MySql),
                                    Kind::Error,
                                    format!("Installing phpMyAdmin failed: {err}"),
                                ),
                            }
                        }
                    }
                    cx.notify();
                });
                if done || ok.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    pub fn clear_log(&mut self, cx: &mut Context<Self>) {
        self.log.clear();
        cx.notify();
    }

    fn on_settings_event(
        &mut self,
        _: &Entity<SettingsView>,
        event: &SettingsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SettingsEvent::Save(settings) = event;
        // The running servers' pid files live in the old folder; moving away
        // from them would lose track of the servers.
        if settings.root() != self.stack.settings().root() && self.any_running() {
            let text = "Stop the servers before moving the stack folder";
            self.log.push(None, Kind::Error, text);
            window.push_notification(Notification::error(text), cx);
            cx.notify();
            return;
        }
        if let Err(err) = settings.save(&self.settings_path) {
            self.log.push(
                None,
                Kind::Error,
                format!("Saving settings failed: {err:#}"),
            );
            cx.notify();
            return;
        }
        let old = self.stack.settings().clone();
        self.stack = Arc::new(self.stack.reconfigure(settings.clone()));
        self.log.push(None, Kind::Ok, "Settings saved");
        if old.root != settings.root {
            self.log.push(
                None,
                Kind::Info,
                format!("Stack root: {}", self.stack.layout().root().display()),
            );
        }
        if old.ports != settings.ports || old.binaries != settings.binaries {
            for m in Module::ALL {
                if self.row(m).running() {
                    self.log.push(
                        Some(m),
                        Kind::Info,
                        format!("Restart {m} to use the new settings"),
                    );
                }
            }
        }
        let stack = self.stack.clone();
        self.settings_view
            .update(cx, |view, cx| view.reload(&stack, window, cx));
        window.push_notification(Notification::success("Settings saved"), cx);
        cx.notify();
    }
}

fn progress_text(progress: &Progress) -> String {
    match progress {
        Progress::Checking => "Looking up the latest version…".into(),
        Progress::Downloading {
            version,
            fraction: Some(f),
        } => format!("Downloading {version}… {:.0}%", f * 100.0),
        Progress::Downloading { version, .. } => format!("Downloading {version}…"),
        Progress::Unpacking { version } => format!("Unpacking {version}…"),
    }
}

/// Follows the system, unless `SLOMPP_THEME=light|dark` says otherwise.
pub fn wants_light(appearance: WindowAppearance) -> bool {
    match std::env::var("SLOMPP_THEME").as_deref() {
        Ok("light") => true,
        Ok("dark") => false,
        _ => matches!(
            appearance,
            WindowAppearance::Light | WindowAppearance::VibrantLight
        ),
    }
}

impl Focusable for Panel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The console follows new lines.
        if self.scrolled_revision != self.log.revision {
            self.scrolled_revision = self.log.revision;
            self.log_scroll.scroll_to_bottom();
        }
        let body = match self.page {
            Page::Modules => v_flex()
                .flex_1()
                .min_h_0()
                .p_4()
                .gap_3()
                .child(rack::render(self, window, cx))
                .child(rack::render_stack_bar(self, cx))
                .child(console::render(self, cx))
                .into_any_element(),
            Page::Ports => ports::render(self, cx).into_any_element(),
            Page::Settings => div()
                .flex_1()
                .min_h_0()
                .child(self.settings_view.clone())
                .into_any_element(),
        };

        v_flex()
            .id("panel")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::start_all))
            .on_action(cx.listener(Self::stop_all))
            .on_action(cx.listener(|this, _: &ShowModules, _, cx| this.show(Page::Modules, cx)))
            .on_action(cx.listener(|this, _: &ShowPorts, _, cx| this.show(Page::Ports, cx)))
            .on_action(cx.listener(|this, _: &ShowSettings, _, cx| this.show(Page::Settings, cx)))
            .size_full()
            .bg(Palette::bg())
            .text_color(Palette::text())
            .font_family(theme::FONT)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .h_full()
                        .pr_2()
                        .justify_between()
                        .child(brand::render_nav(self, cx))
                        .child(brand::render_global_actions(self, cx)),
                ),
            )
            .child(body)
    }
}
