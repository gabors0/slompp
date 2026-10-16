//! The app's state, independent of any window: owns the stack, watches the
//! modules, runs what the buttons and the tray ask for, and keeps the log.
//! The panel window and the tray icon are both views of it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task};
use slopp_core::dbadmin::{Installed, Progress};
use slopp_core::install::{self, Installer};
use slopp_core::net::Listener;
use slopp_core::update::{self, Install, Release, Restart};
use slopp_core::{DbAdmin, Module, ModuleStatus, Problem, Settings, Stack, desktop};

use crate::log::{Kind, Log};
use crate::panel;
use crate::theme::Lamp;
use crate::tray::{self, TrayHandle, TrayModule, TrayState};

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
    /// When the last start or stop finished. A poll begun before then saw
    /// the module as it was (no pid file yet, or still running), so it is
    /// stale for this row.
    settled: Instant,
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

    /// Takes in a poll of this module begun at `taken`, moves the phase on,
    /// and says what is worth logging. `now` is when the poll came back.
    fn observe(
        &mut self,
        status: ModuleStatus,
        problems: Vec<Problem>,
        taken: Instant,
        now: Instant,
    ) -> Vec<Seen> {
        // Begun before a start or stop finished: it would read a module
        // that was just launched as gone, or one just stopped as running.
        if taken < self.settled {
            return Vec::new();
        }
        let was_running = self.status.as_ref().map(ModuleStatus::running);
        let mut seen = Vec::new();
        match self.phase {
            Phase::Starting {
                launched: Some(launched),
            } => {
                if status.ready() {
                    self.phase = Phase::Idle;
                    seen.push(Seen::Ready);
                } else if !status.running() {
                    self.phase = Phase::Failed;
                    seen.push(Seen::DiedStarting);
                } else if now.saturating_duration_since(launched) > START_TIMEOUT {
                    self.phase = Phase::Failed;
                    seen.push(Seen::TimedOut);
                }
            }
            Phase::Starting { launched: None } | Phase::Stopping => {}
            Phase::Idle | Phase::Failed => {
                if was_running == Some(true) && !status.running() {
                    self.phase = Phase::Failed;
                    seen.push(Seen::DiedUnexpectedly);
                } else if was_running == Some(false) && status.running() {
                    self.phase = Phase::Idle;
                    seen.push(Seen::StartedOutside);
                } else if was_running.is_none() && status.running() {
                    seen.push(Seen::AlreadyRunning);
                } else if self.phase == Phase::Failed && status.ready() {
                    // Opened its ports after the start timed out: slow
                    // (a first start, a busy disk), not broken.
                    self.phase = Phase::Idle;
                    seen.push(Seen::Ready);
                }
            }
        }
        // Report a port conflict once, when it appears.
        seen.extend(
            problems
                .iter()
                .filter(|p| matches!(p, Problem::PortInUse { .. }) && !self.problems.contains(p))
                .map(|p| Seen::Conflict(p.to_string())),
        );
        self.status = Some(status);
        self.problems = problems;
        seen
    }
}

/// What a poll showed about a module that the log should say.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    /// Its ports opened after a start (or after the start timed out).
    Ready,
    /// It exited before opening its ports.
    DiedStarting,
    /// It runs, but didn't open its ports within `START_TIMEOUT`.
    TimedOut,
    /// It was running and has exited without being asked to.
    DiedUnexpectedly,
    /// It was stopped and runs now, started by something else.
    StartedOutside,
    /// It was already running when the panel first looked.
    AlreadyRunning,
    /// A port conflict that wasn't there the last time.
    Conflict(String),
}

/// "Running on ports 8080 and 8443".
fn running_text(status: &ModuleStatus) -> String {
    let ports: Vec<String> = status.ports.iter().map(|p| p.port.to_string()).collect();
    let what = if ports.len() == 1 { "port" } else { "ports" };
    format!("Running on {what} {}", ports.join(" and "))
}

/// Where updating SLOPP itself stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    /// Not looked for yet, or checks are off.
    Unknown,
    Checking,
    UpToDate,
    Available(Release),
    /// Downloading it and putting it in place; `progress` says how far.
    Installing {
        release: Release,
        progress: String,
    },
    /// In place (for the Windows installer, downloaded): a restart uses it.
    Ready {
        version: String,
        restart: Restart,
    },
    /// The check failed, or with `release`, the update did.
    Failed {
        error: String,
        release: Option<Release>,
    },
}

/// Something to tell the user in the window, if one is open.
pub enum ControllerEvent {
    Notify { text: String, error: bool },
}

pub struct Controller {
    stack: Arc<Stack>,
    settings_path: PathBuf,
    pub rows: Vec<Row>,
    pub log: Log,
    pub listeners: Vec<Listener>,
    /// A database admin tool being downloaded, and its progress for the
    /// MySQL row. One at a time.
    pub installing: Option<(DbAdmin, String)>,
    /// A module whose servers the package manager is installing, and the
    /// line it printed last. One module at a time.
    pub getting: Option<(Module, String)>,
    /// Whether a window shows the Ports page (polls then list listeners).
    pub watch_ports: bool,
    /// How this SLOPP was installed, which says how it updates.
    pub install: Install,
    pub update: UpdateState,
    /// The release a window last announced, so each is announced once.
    pub announced: Option<String>,
    /// Set as SLOPP quits to restart into an update.
    restarting: bool,
    tray: Option<TrayHandle>,
    /// The state last sent to the tray.
    tray_state: Option<TrayState>,
    _poll: Task<()>,
    _update_checks: Task<()>,
}

/// One poll: each module's status and problems, and the listening ports
/// when the Ports page is open.
/// When the poll began, what it found per module, and the listeners if asked.
type Snapshot = (
    Instant,
    Vec<(ModuleStatus, Vec<Problem>)>,
    Option<Vec<Listener>>,
);

/// How long a launched module gets to open its ports.
const START_TIMEOUT: Duration = Duration::from_secs(45);

/// How often a running SLOPP looks for a new release.
const UPDATE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// The version updates are compared with: this one's, unless
/// `SLOPP_UPDATE_FROM` pretends to be another (to try the update flow).
fn current_version() -> String {
    std::env::var("SLOPP_UPDATE_FROM").unwrap_or_else(|_| update::CURRENT.to_string())
}

/// Lets the tray and the window find the controller.
struct ControllerHandle(Entity<Controller>);

impl Global for ControllerHandle {}

impl EventEmitter<ControllerEvent> for Controller {}

impl Controller {
    /// Creates the controller and makes it reachable through [`Controller::global`].
    pub fn init(cx: &mut App) -> Entity<Controller> {
        let entity = cx.new(Controller::new);
        cx.set_global(ControllerHandle(entity.clone()));
        entity
    }

    pub fn global(cx: &App) -> Entity<Controller> {
        cx.global::<ControllerHandle>().0.clone()
    }

    fn new(cx: &mut Context<Self>) -> Self {
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

        cx.on_app_quit(|this, _| {
            let stack = this.stack.clone();
            // An update's restart leaves them for the new SLOPP.
            let stop = stack.settings().stop_on_exit && !this.restarting;
            async move {
                if stop {
                    for m in Module::ALL {
                        let _ = stack.stop(m);
                    }
                }
            }
        })
        .detach();

        let rows = Module::ALL
            .into_iter()
            .map(|module| Row {
                module,
                status: None,
                problems: Vec::new(),
                phase: Phase::Idle,
                settled: Instant::now(),
            })
            .collect();

        let mut this = Self {
            stack,
            settings_path,
            rows,
            log,
            listeners: Vec::new(),
            installing: None,
            getting: None,
            watch_ports: false,
            install: Install::detect(),
            update: UpdateState::Unknown,
            announced: None,
            restarting: false,
            tray: None,
            tray_state: None,
            _poll: Task::ready(()),
            _update_checks: Task::ready(()),
        };
        this.greet();
        this._poll = this.poll_loop(cx);
        this._update_checks = this.update_loop(cx);
        this.autostart(cx);
        this
    }

    pub fn stack(&self) -> &Arc<Stack> {
        &self.stack
    }

    pub fn settings(&self) -> &Settings {
        self.stack.settings()
    }

    pub fn row(&self, module: Module) -> &Row {
        self.rows.iter().find(|r| r.module == module).unwrap()
    }

    fn row_mut(&mut self, module: Module) -> &mut Row {
        self.rows.iter_mut().find(|r| r.module == module).unwrap()
    }

    pub fn any_running(&self) -> bool {
        self.rows.iter().any(Row::running)
    }

    pub fn all_running(&self) -> bool {
        self.rows.iter().all(Row::running)
    }

    pub fn has_tray(&self) -> bool {
        self.tray.is_some()
    }

    /// Takes the icon away, as the app quits: on Windows it would otherwise
    /// linger on the taskbar until the mouse passes over it.
    pub fn remove_tray(&mut self) {
        self.tray = None;
    }

    pub fn set_tray(&mut self, tray: TrayHandle, cx: &mut Context<Self>) {
        self.tray = Some(tray);
        self.tray_state = None;
        self.changed(cx);
    }

    /// Redraws the views and brings the tray up to date.
    fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(tray) = &self.tray {
            let state = self.tray_snapshot(cx);
            if self.tray_state.as_ref() != Some(&state) {
                tray.set(state.clone());
                self.tray_state = Some(state);
            }
        }
        cx.notify();
    }

    pub fn tray_snapshot(&self, cx: &App) -> TrayState {
        let modules = self
            .rows
            .iter()
            .map(|row| TrayModule {
                module: row.module,
                lamp: row.lamp(),
                running: row.running(),
                busy: row.busy(),
                label: row.state_label(),
                ports: row
                    .status
                    .as_ref()
                    .filter(|s| s.running())
                    .map(|s| s.ports.iter().map(|p| p.port).collect())
                    .unwrap_or_default(),
            })
            .collect();
        TrayState {
            modules,
            db_admins: DbAdmin::ALL
                .into_iter()
                .filter(|&tool| self.stack.db_admin_url(tool).is_some())
                .collect(),
            light_panel: tray::light_panel(panel::system_light(cx.window_appearance())),
        }
    }

    /// The opening lines: where the stack is and what runs it.
    fn greet(&mut self) {
        let root = self.stack.layout().root().display().to_string();
        self.log.push(
            None,
            Kind::Info,
            format!("SLOPP {}", env!("CARGO_PKG_VERSION")),
        );
        self.log
            .push(None, Kind::Info, format!("Stack root: {root}"));
        let crash_log = crate::crash::log_path();
        if let Some(crash) = crate::crash::take_last(&crash_log) {
            self.log.push(
                None,
                Kind::Error,
                format!(
                    "The last run crashed: {crash} (details in {})",
                    crash_log.with_extension("log.old").display()
                ),
            );
        }
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
                        this.watch_ports,
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
                        let taken = Instant::now();
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
                        let listeners = want_ports.then(slopp_core::net::listeners);
                        (taken, modules, listeners)
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

    fn apply_snapshot(&mut self, (taken, modules, listeners): Snapshot, cx: &mut Context<Self>) {
        let now = Instant::now();
        for (status, problems) in modules {
            let m = status.module;
            for seen in self.row_mut(m).observe(status, problems, taken, now) {
                let (kind, text) = match seen {
                    Seen::Ready => {
                        let status = self.row(m).status.as_ref();
                        (Kind::Ok, running_text(status.expect("just observed")))
                    }
                    Seen::DiedStarting => (
                        Kind::Error,
                        format!(
                            "{m} stopped while starting.\n{}",
                            self.stack.failure_details(m)
                        ),
                    ),
                    Seen::TimedOut => (
                        Kind::Error,
                        format!(
                            "{m} is running but did not open its ports within {}s. \
                             If it is only slow, it turns green when it does.",
                            START_TIMEOUT.as_secs()
                        ),
                    ),
                    Seen::DiedUnexpectedly => (
                        Kind::Error,
                        format!(
                            "{m} stopped unexpectedly.\n{}",
                            self.stack.failure_details(m)
                        ),
                    ),
                    Seen::StartedOutside => {
                        (Kind::Info, format!("{m} was started outside the panel"))
                    }
                    Seen::AlreadyRunning => (Kind::Info, format!("{m} is already running")),
                    Seen::Conflict(text) => (Kind::Error, text),
                };
                self.log.push(Some(m), kind, text);
            }
        }
        if let Some(listeners) = listeners {
            self.listeners = listeners;
        }
        self.changed(cx);
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
            module == Module::MySql && !slopp_core::setup::mysql_initialised(self.stack.layout());
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
                this.row_mut(module).settled = Instant::now();
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
                this.changed(cx);
            })
            .ok();
        })
        .detach();
        self.changed(cx);
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
                let row = this.row_mut(module);
                row.settled = Instant::now();
                if let Some(status) = &mut row.status {
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
                this.changed(cx);
            })
            .ok();
        })
        .detach();
        self.changed(cx);
    }

    pub fn start_all(&mut self, cx: &mut Context<Self>) {
        for m in Module::ALL {
            if !self.row(m).running() {
                self.start(m, cx);
            }
        }
    }

    pub fn stop_all(&mut self, cx: &mut Context<Self>) {
        for m in Module::ALL {
            if self.row(m).running() {
                self.stop(m, cx);
            }
        }
    }

    pub fn refresh_ports(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let listeners = cx
                .background_spawn(async { slopp_core::net::listeners() })
                .await;
            this.update(cx, |this, cx| {
                this.listeners = listeners;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn report(&mut self, result: anyhow::Result<()>, cx: &mut Context<Self>) {
        if let Err(err) = result {
            self.log.push(None, Kind::Error, format!("{err:#}"));
            cx.notify();
        }
    }

    /// Opens a config or log file in the editor from Settings.
    pub fn open_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let editor = self.stack.settings().editor.clone();
        let result = desktop::open_path(&path, editor.as_deref());
        self.report(result, cx);
    }

    pub fn open_site(&mut self, cx: &mut Context<Self>) {
        let url = self.stack.site_url();
        self.open_served(Module::Apache, url, cx);
    }

    pub fn open_db_admin(&mut self, tool: DbAdmin, cx: &mut Context<Self>) {
        if let Some(url) = self.stack.db_admin_url(tool) {
            self.open_served(Module::MySql, url, cx);
        }
    }

    /// Mailpit's page, which Mailpit serves itself.
    pub fn open_mailpit(&mut self, cx: &mut Context<Self>) {
        if !self.row(Module::Mailpit).running() {
            self.log.push(
                Some(Module::Mailpit),
                Kind::Info,
                "Start Mailpit first: it serves its own page.",
            );
            cx.notify();
            return;
        }
        let result = desktop::open_url(&self.stack.mailpit_url());
        self.report(result, cx);
    }

    /// Opens a page Apache serves, or says to start Apache.
    fn open_served(&mut self, module: Module, url: String, cx: &mut Context<Self>) {
        if !self.row(Module::Apache).running() {
            self.log.push(
                Some(module),
                Kind::Info,
                "Start Apache first: the admin pages are served by it.",
            );
            cx.notify();
            return;
        }
        let result = desktop::open_url(&url);
        self.report(result, cx);
    }

    pub fn open_root(&mut self, cx: &mut Context<Self>) {
        let root = self.stack.layout().root().to_path_buf();
        self.open_folder(root, cx);
    }

    /// Opens a folder in the file manager, creating it if needed.
    pub fn open_folder(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let result = std::fs::create_dir_all(&dir)
            .map_err(anyhow::Error::from)
            .and_then(|_| desktop::open_path(&dir, None));
        self.report(result, cx);
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

    pub fn install_db_admin(&mut self, tool: DbAdmin, cx: &mut Context<Self>) {
        if self.installing.is_some() {
            return;
        }
        let name = tool.name();
        self.installing = Some((tool, "Looking up the latest version…".into()));
        self.log.push(
            Some(Module::MySql),
            Kind::Info,
            format!("Installing {name}…"),
        );
        let layout = self.stack.layout().clone();
        let port = self.stack.settings().ports.mysql;
        type Done = Result<Installed, String>;
        let (tx, rx) = async_channel::unbounded::<Result<Progress, Done>>();
        std::thread::spawn(move || {
            let result = tool.install(&layout, port, |p| {
                let _ = tx.send_blocking(Ok(p));
            });
            let _ = tx.send_blocking(Err(result.map_err(|e| format!("{e:#}"))));
        });
        cx.spawn(async move |this, cx| {
            while let Ok(message) = rx.recv().await {
                let done = message.is_err();
                let ok = this.update(cx, |this, cx| {
                    match message {
                        Ok(progress) => this.installing = Some((tool, progress_text(&progress))),
                        Err(result) => {
                            this.installing = None;
                            match result {
                                Ok(installed) => {
                                    let text = format!("{name} {} is installed", installed.version);
                                    this.log.push(Some(Module::MySql), Kind::Ok, text.clone());
                                    if installed.apache_conf_changed
                                        && this.row(Module::Apache).running()
                                    {
                                        this.log.push(
                                            Some(Module::Apache),
                                            Kind::Info,
                                            format!("Restart Apache to serve {name}"),
                                        );
                                    }
                                    cx.emit(ControllerEvent::Notify { text, error: false });
                                }
                                Err(err) => this.log.push(
                                    Some(Module::MySql),
                                    Kind::Error,
                                    format!("Installing {name} failed: {err}"),
                                ),
                            }
                        }
                    }
                    this.changed(cx);
                });
                if done || ok.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// Installs the servers `module` is missing with the system's package
    /// manager (winget, Homebrew, apt…), then looks for the programs again.
    pub fn install_servers(&mut self, module: Module, cx: &mut Context<Self>) {
        if self.getting.is_some() {
            return;
        }
        let packages = self.stack.installable(module);
        if packages.is_empty() {
            return;
        }
        let installer = match Installer::find(&packages, self.stack.layout()) {
            Ok(installer) => installer,
            Err(unavailable) => {
                self.log
                    .push(Some(module), Kind::Error, unavailable.message);
                cx.emit(ControllerEvent::Notify {
                    text: unavailable.notice,
                    error: true,
                });
                cx.notify();
                return;
            }
        };
        let name = installer.name();
        self.log.push(
            Some(module),
            Kind::Info,
            format!(
                "Installing {} {}…",
                install::names(&packages),
                installer.via()
            ),
        );
        self.getting = Some((module, format!("Starting {name}…")));

        enum Message {
            Line(String),
            Done(Result<(), String>),
        }
        let (tx, rx) = async_channel::unbounded::<Message>();
        std::thread::spawn(move || {
            let result = installer
                .install(&packages, |line| {
                    let _ = tx.send_blocking(Message::Line(line));
                })
                .map_err(|err| format!("{err:#}"));
            let _ = tx.send_blocking(Message::Done(result));
        });
        cx.spawn(async move |this, cx| {
            while let Ok(message) = rx.recv().await {
                let done = matches!(message, Message::Done(_));
                let ok = this.update(cx, |this, cx| {
                    match message {
                        Message::Line(line) => {
                            this.getting = Some((module, line));
                        }
                        Message::Done(result) => {
                            this.getting = None;
                            this.servers_installed(module, result, cx);
                        }
                    }
                    this.changed(cx);
                });
                if done || ok.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// After the package manager: finds the new programs and says what runs the module.
    fn servers_installed(
        &mut self,
        module: Module,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        // Whatever failed, an earlier package may have gone in.
        let settings = self.stack.settings().clone();
        self.stack = Arc::new(self.stack.reconfigure(settings));
        if let Err(errors) = result {
            for err in errors.lines() {
                self.log.push(Some(module), Kind::Error, err.to_string());
            }
            if let Some(text) = self.stack.describe(module) {
                self.log
                    .push(Some(module), Kind::Info, format!("Found: {text}"));
            }
            return;
        }
        match self.stack.describe(module) {
            Some(text) if self.stack.installable(module).is_empty() => {
                self.log
                    .push(Some(module), Kind::Ok, format!("Installed: {text}"));
                cx.emit(ControllerEvent::Notify {
                    text: format!("{module} is installed"),
                    error: false,
                });
            }
            _ => {
                // The package manager succeeded, yet the programs aren't where
                // we look.
                for problem in self.stack.problems(module) {
                    if let Problem::Missing { .. } = problem {
                        self.log
                            .push(Some(module), Kind::Error, problem.to_string());
                    }
                }
            }
        }
    }

    /// Tidies up after an earlier update, then looks for a new release
    /// shortly after the start and daily, while Settings allows it.
    fn update_loop(&self, cx: &mut Context<Self>) -> Task<()> {
        let install = self.install.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            // The old version may only just have quit.
            cx.background_spawn(async move { install.clean_up() }).await;
            loop {
                let checked = this.update(cx, |this, cx| {
                    if this.settings().check_updates {
                        this.check_for_update(false, cx);
                    }
                });
                if checked.is_err() {
                    return;
                }
                cx.background_executor().timer(UPDATE_INTERVAL).await;
            }
        })
    }

    /// Asks GitHub for the latest release. `manual` (the Check button) also
    /// reports when there's nothing new, or the check failed, in the log.
    pub fn check_for_update(&mut self, manual: bool, cx: &mut Context<Self>) {
        let previous = self.update.clone();
        if matches!(
            previous,
            UpdateState::Checking | UpdateState::Installing { .. } | UpdateState::Ready { .. }
        ) {
            return;
        }
        self.update = UpdateState::Checking;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async { update::latest().map_err(|err| format!("{err:#}")) })
                .await;
            this.update(cx, |this, cx| {
                let (state, line) = checked(previous, result, &current_version(), manual);
                if let Some((kind, text)) = line {
                    this.log.push(None, kind, text);
                }
                this.update = state;
                this.changed(cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Downloads the available release and puts it in place of this SLOPP.
    pub fn install_update(&mut self, cx: &mut Context<Self>) {
        let release = match &self.update {
            UpdateState::Available(release)
            | UpdateState::Failed {
                release: Some(release),
                ..
            } => release.clone(),
            _ => return,
        };
        if !self.install.can_update() {
            return;
        }
        let version = release.version.clone();
        self.log
            .push(None, Kind::Info, format!("Updating SLOPP to {version}…"));
        self.update = UpdateState::Installing {
            release: release.clone(),
            progress: "Downloading…".into(),
        };
        enum Message {
            Progress(String),
            Done(Result<Restart, String>),
        }
        let (tx, rx) = async_channel::unbounded::<Message>();
        let install = self.install.clone();
        let wanted = release.clone();
        std::thread::spawn(move || {
            let result = install.install(&wanted, |p| {
                let text = match p {
                    Progress::Downloading {
                        fraction: Some(f), ..
                    } => format!("Downloading… {:.0}%", f * 100.0),
                    Progress::Checking | Progress::Downloading { .. } => "Downloading…".into(),
                    Progress::Unpacking { .. } => "Installing…".into(),
                };
                let _ = tx.send_blocking(Message::Progress(text));
            });
            let _ = tx.send_blocking(Message::Done(result.map_err(|e| format!("{e:#}"))));
        });
        cx.spawn(async move |this, cx| {
            while let Ok(message) = rx.recv().await {
                let done = matches!(message, Message::Done(_));
                let ok = this.update(cx, |this, cx| {
                    match message {
                        Message::Progress(progress) => {
                            this.update = UpdateState::Installing {
                                release: release.clone(),
                                progress,
                            };
                        }
                        Message::Done(Ok(restart)) => {
                            #[cfg(target_os = "linux")]
                            let restart = match crate::appimage::launcher() {
                                Some(launcher) if matches!(this.install, Install::AppImage(_)) => {
                                    restart.through(launcher)
                                }
                                _ => restart,
                            };
                            let text = if restart.installs {
                                format!(
                                    "SLOPP {version} is downloaded: restart SLOPP to install it"
                                )
                            } else {
                                format!("SLOPP {version} is installed: restart SLOPP to use it")
                            };
                            this.log.push(None, Kind::Ok, text);
                            this.update = UpdateState::Ready {
                                version: version.clone(),
                                restart,
                            };
                        }
                        Message::Done(Err(error)) => {
                            this.log.push(
                                None,
                                Kind::Error,
                                format!("Updating SLOPP failed: {error}"),
                            );
                            this.update = UpdateState::Failed {
                                error,
                                release: Some(release.clone()),
                            };
                        }
                    }
                    this.changed(cx);
                });
                if done || ok.is_err() {
                    break;
                }
            }
        })
        .detach();
        self.changed(cx);
    }

    /// Quits and starts the update (or the Windows installer, which starts
    /// it). The servers keep running for the new SLOPP.
    pub fn restart_into_update(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Ready { restart, .. } = self.update.clone() else {
            return;
        };
        // The new SLOPP must not find this one still answering, and this one
        // must not remove the new one's socket as it quits.
        crate::instance::release();
        match restart.spawn() {
            Ok(()) => {
                self.restarting = true;
                cx.quit();
            }
            Err(err) => {
                self.log.push(
                    None,
                    Kind::Error,
                    format!("Restarting SLOPP failed: {err:#}"),
                );
                cx.notify();
            }
        }
    }

    /// The available release's page, or the releases page.
    pub fn open_release_page(&mut self, cx: &mut Context<Self>) {
        let page = match &self.update {
            UpdateState::Available(release)
            | UpdateState::Installing { release, .. }
            | UpdateState::Failed {
                release: Some(release),
                ..
            } => release.page.clone(),
            _ => update::RELEASES_PAGE.to_string(),
        };
        let result = desktop::open_url(&page);
        self.report(result, cx);
    }

    pub fn clear_log(&mut self, cx: &mut Context<Self>) {
        self.log.clear();
        cx.notify();
    }

    /// Saves and applies new settings. The error is for the user.
    pub fn apply_settings(
        &mut self,
        settings: Settings,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        // The running servers' pid files live in the old folder; moving away
        // from them would lose track of the servers.
        if settings.root() != self.stack.settings().root() && self.any_running() {
            let text = "Stop the servers before moving the stack folder".to_string();
            self.log.push(None, Kind::Error, text.clone());
            cx.notify();
            return Err(text);
        }
        if let Err(err) = settings.save(&self.settings_path) {
            let text = format!("Saving settings failed: {err:#}");
            self.log.push(None, Kind::Error, text.clone());
            cx.notify();
            return Err(text);
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
        self.changed(cx);
        Ok(())
    }

    /// Saves the window style, quietly: it isn't one of the form's settings.
    pub fn set_compact(&mut self, compact: bool, cx: &mut Context<Self>) -> Result<(), String> {
        let mut settings = self.stack.settings().clone();
        settings.compact = compact;
        if let Err(err) = settings.save(&self.settings_path) {
            let text = format!("Saving settings failed: {err:#}");
            self.log.push(None, Kind::Error, text.clone());
            // The compact window has no log to read it in.
            cx.emit(ControllerEvent::Notify {
                text: text.clone(),
                error: true,
            });
            cx.notify();
            return Err(text);
        }
        self.stack = Arc::new(self.stack.reconfigure(settings));
        cx.notify();
        Ok(())
    }
}

/// What a finished update check, begun in state `previous`, makes the state,
/// and the line to log if any. `manual` (the Check button) also reports
/// when there's nothing new, or the check failed.
fn checked(
    previous: UpdateState,
    result: Result<Release, String>,
    current: &str,
    manual: bool,
) -> (UpdateState, Option<(Kind, String)>) {
    match result {
        Ok(release) if update::is_newer(&release.version, current) => {
            // Each daily check finds it again; say so once.
            let line = (previous != UpdateState::Available(release.clone())).then(|| {
                (
                    Kind::Info,
                    format!("SLOPP {} is available", release.version),
                )
            });
            (UpdateState::Available(release), line)
        }
        Ok(_) => (
            UpdateState::UpToDate,
            manual.then(|| (Kind::Ok, "SLOPP is up to date".to_string())),
        ),
        Err(error) if manual => (
            UpdateState::Failed {
                error: error.clone(),
                release: None,
            },
            Some((Kind::Error, format!("Checking for updates failed: {error}"))),
        ),
        Err(error) => {
            // Offline, most likely; the next check may do.
            tracing::info!("checking for updates failed: {error}");
            (previous, None)
        }
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

#[cfg(test)]
mod tests {
    use slopp_core::stack::PortState;

    use super::*;

    fn row(phase: Phase, status: Option<ModuleStatus>) -> Row {
        Row {
            module: Module::Apache,
            status,
            problems: Vec::new(),
            phase,
            settled: Instant::now(),
        }
    }

    /// Apache with `pids` processes and its HTTP port open or not.
    fn apache(pids: &[u32], open: bool) -> ModuleStatus {
        ModuleStatus {
            module: Module::Apache,
            pids: pids.to_vec(),
            ports: vec![PortState {
                port: 8080,
                label: "HTTP",
                open,
            }],
        }
    }

    fn launched(at: Instant) -> Phase {
        Phase::Starting { launched: Some(at) }
    }

    /// Observes `status` with a poll begun just now, `after` the launch.
    fn poll(row: &mut Row, status: ModuleStatus, after: Duration) -> Vec<Seen> {
        let taken = Instant::now();
        let now = match row.phase {
            Phase::Starting { launched: Some(at) } => at + after,
            _ => taken,
        };
        row.observe(status, Vec::new(), taken, now)
    }

    #[test]
    fn a_start_turns_green_when_the_ports_open() {
        let mut r = row(launched(Instant::now()), None);
        assert_eq!(poll(&mut r, apache(&[1], false), Duration::ZERO), []);
        assert_eq!(r.lamp(), Lamp::Busy);
        assert_eq!(
            poll(&mut r, apache(&[1], true), Duration::ZERO),
            [Seen::Ready]
        );
        assert_eq!(
            (r.phase, r.lamp(), r.state_label()),
            (Phase::Idle, Lamp::On, "Running")
        );
        // Nothing more to say while it stays up.
        assert_eq!(poll(&mut r, apache(&[1], true), Duration::ZERO), []);
    }

    #[test]
    fn a_server_that_exits_while_starting_failed() {
        let mut r = row(launched(Instant::now()), None);
        assert_eq!(
            poll(&mut r, apache(&[], false), Duration::ZERO),
            [Seen::DiedStarting]
        );
        assert_eq!(r.phase, Phase::Failed);
        assert_eq!(
            (r.lamp(), r.state_label()),
            (Lamp::Fault, "Stopped with an error")
        );
    }

    #[test]
    fn a_slow_start_times_out_then_turns_green() {
        let mut r = row(launched(Instant::now()), None);
        let late = START_TIMEOUT + Duration::from_secs(1);
        assert_eq!(poll(&mut r, apache(&[1], false), START_TIMEOUT), []);
        assert_eq!(poll(&mut r, apache(&[1], false), late), [Seen::TimedOut]);
        assert_eq!(r.phase, Phase::Failed);
        // Still not open: the error isn't repeated.
        assert_eq!(poll(&mut r, apache(&[1], false), late), []);
        assert_eq!(poll(&mut r, apache(&[1], true), late), [Seen::Ready]);
        assert_eq!(r.phase, Phase::Idle);
    }

    #[test]
    fn setup_and_stopping_wait_for_their_own_result() {
        for phase in [Phase::Starting { launched: None }, Phase::Stopping] {
            let mut r = row(phase, Some(apache(&[1], true)));
            assert_eq!(poll(&mut r, apache(&[], false), Duration::ZERO), []);
            assert_eq!(r.phase, phase);
            assert!(r.busy());
        }
    }

    #[test]
    fn an_exit_nobody_asked_for_is_a_failure() {
        let mut r = row(Phase::Idle, Some(apache(&[1], true)));
        assert_eq!(
            poll(&mut r, apache(&[], false), Duration::ZERO),
            [Seen::DiedUnexpectedly]
        );
        assert_eq!(r.phase, Phase::Failed);
        // Reported once.
        assert_eq!(poll(&mut r, apache(&[], false), Duration::ZERO), []);
    }

    #[test]
    fn servers_started_elsewhere_are_noticed() {
        // Running before the panel's first look.
        let mut r = row(Phase::Idle, None);
        assert_eq!(
            poll(&mut r, apache(&[1], true), Duration::ZERO),
            [Seen::AlreadyRunning]
        );
        assert_eq!(r.phase, Phase::Idle);

        // Started later, by sloppctl say, also after a failed start.
        for phase in [Phase::Idle, Phase::Failed] {
            let mut r = row(phase, Some(apache(&[], false)));
            assert_eq!(
                poll(&mut r, apache(&[1], true), Duration::ZERO),
                [Seen::StartedOutside]
            );
            assert_eq!(r.phase, Phase::Idle);
        }
    }

    #[test]
    fn a_poll_begun_before_a_start_or_stop_finished_is_ignored() {
        let taken = Instant::now();
        // The stop finished after the poll began, and found it running.
        let mut r = row(Phase::Idle, Some(apache(&[], false)));
        r.settled = taken + Duration::from_millis(1);
        let seen = r.observe(apache(&[1], true), Vec::new(), taken, r.settled);
        assert_eq!(seen, []);
        assert!(!r.running());
    }

    #[test]
    fn port_conflicts_are_reported_once() {
        let taken = Problem::PortInUse {
            port: 8080,
            owner: None,
        };
        let mut r = row(Phase::Idle, Some(apache(&[], false)));
        let now = Instant::now();
        let seen = r.observe(apache(&[], false), vec![taken.clone()], now, now);
        assert_eq!(seen, [Seen::Conflict(taken.to_string())]);
        let seen = r.observe(apache(&[], false), vec![taken.clone()], now, now);
        assert_eq!(seen, []);
        // Freed, then taken again: a new conflict.
        r.observe(apache(&[], false), Vec::new(), now, now);
        let seen = r.observe(apache(&[], false), vec![taken.clone()], now, now);
        assert_eq!(seen, [Seen::Conflict(taken.to_string())]);
    }

    #[test]
    fn running_text_names_the_ports() {
        let mut status = apache(&[1], true);
        assert_eq!(running_text(&status), "Running on port 8080");
        status.ports.push(PortState {
            port: 8443,
            label: "HTTPS",
            open: true,
        });
        assert_eq!(running_text(&status), "Running on ports 8080 and 8443");
    }

    fn release(version: &str) -> Release {
        update::parse_release(&format!(
            r#"{{"tag_name": "v{version}", "html_url": "https://example.com/v{version}", "assets": []}}"#
        ))
        .unwrap()
    }

    #[test]
    fn a_newer_release_is_announced_once() {
        let (state, line) = checked(UpdateState::Unknown, Ok(release("0.4.0")), "0.3.1", false);
        assert_eq!(state, UpdateState::Available(release("0.4.0")));
        assert_eq!(line, Some((Kind::Info, "SLOPP 0.4.0 is available".into())));
        // The next day's check finds the same one.
        let (state, line) = checked(state, Ok(release("0.4.0")), "0.3.1", false);
        assert_eq!(state, UpdateState::Available(release("0.4.0")));
        assert_eq!(line, None);
        // And a newer one again.
        let (_, line) = checked(state, Ok(release("0.4.1")), "0.3.1", false);
        assert_eq!(line, Some((Kind::Info, "SLOPP 0.4.1 is available".into())));
    }

    #[test]
    fn up_to_date_is_only_said_when_asked() {
        for current in ["0.3.1", "0.4.0-dev"] {
            let (state, line) = checked(UpdateState::Unknown, Ok(release("0.3.1")), current, false);
            assert_eq!((state, line), (UpdateState::UpToDate, None), "{current}");
        }
        let (state, line) = checked(UpdateState::Unknown, Ok(release("0.3.1")), "0.3.1", true);
        assert_eq!(state, UpdateState::UpToDate);
        assert_eq!(line, Some((Kind::Ok, "SLOPP is up to date".into())));
    }

    #[test]
    fn a_failed_check_is_quiet_unless_asked() {
        // Offline at the daily check: whatever was known stays.
        let known = UpdateState::Available(release("0.4.0"));
        let (state, line) = checked(known.clone(), Err("offline".into()), "0.3.1", false);
        assert_eq!((state, line), (known, None));

        let (state, line) = checked(UpdateState::UpToDate, Err("offline".into()), "0.3.1", true);
        assert_eq!(
            state,
            UpdateState::Failed {
                error: "offline".into(),
                release: None
            }
        );
        assert_eq!(
            line,
            Some((Kind::Error, "Checking for updates failed: offline".into()))
        );
    }
}
