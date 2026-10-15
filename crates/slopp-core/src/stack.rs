//! Starting, stopping and watching the modules of one stack root.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};

use crate::install::{self, Package};
use crate::net::{self, Listener};
use crate::process::{self, Tracked};
use crate::templates::conf_path;
use crate::toolchain::PhpHandler;
use crate::{DbAdmin, Layout, Module, Settings, Toolchain, setup, templates};

/// Something that stops a module from starting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// A server program isn't installed (or isn't where settings say).
    Missing { what: &'static str, hint: String },
    /// Another program already listens on a port the module needs.
    PortInUse { port: u16, owner: Option<Listener> },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Missing { what, hint } => write!(f, "{what} was not found. {hint}"),
            Problem::PortInUse { port, owner } => match owner {
                Some(Listener {
                    process: Some(name),
                    pid: Some(pid),
                    ..
                }) => write!(f, "Port {port} is in use by {name} (PID {pid})."),
                _ => write!(f, "Port {port} is in use by another program."),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortState {
    pub port: u16,
    pub label: &'static str,
    pub open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleStatus {
    pub module: Module,
    /// Processes we started that are still alive.
    pub pids: Vec<u32>,
    pub ports: Vec<PortState>,
}

impl ModuleStatus {
    pub fn running(&self) -> bool {
        !self.pids.is_empty()
    }

    /// Running and accepting connections on every port.
    pub fn ready(&self) -> bool {
        self.running() && self.ports.iter().all(|p| p.open)
    }
}

/// A file the panel offers to open, under a name people recognise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedFile {
    pub label: &'static str,
    pub path: PathBuf,
}

fn named(label: &'static str, path: PathBuf) -> NamedFile {
    NamedFile { label, path }
}

pub struct Stack {
    layout: Layout,
    settings: Settings,
    tools: Toolchain,
    /// Processes started by this process, reaped as they exit.
    children: Mutex<Vec<Child>>,
}

impl Stack {
    pub fn new(settings: Settings) -> Self {
        let layout = Layout::new(settings.root());
        let tools = Toolchain::detect(&settings.binaries);
        Self {
            layout,
            settings,
            tools,
            children: Mutex::new(Vec::new()),
        }
    }

    /// The same stack with new settings; children started so far move over.
    pub fn reconfigure(&self, settings: Settings) -> Self {
        let next = Stack::new(settings);
        let children = std::mem::take(&mut *self.children.lock().unwrap());
        *next.children.lock().unwrap() = children;
        next
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn tools(&self) -> &Toolchain {
        &self.tools
    }

    /// Whether Apache can serve HTTPS (mod_ssl is installed).
    pub fn ssl_available(&self) -> bool {
        self.tools
            .httpd
            .as_ref()
            .is_some_and(|h| h.has_module("mod_ssl.so") && h.has_module("mod_socache_shmcb.so"))
    }

    pub fn ports(&self, module: Module) -> Vec<(u16, &'static str)> {
        let ports = self.settings.ports;
        match module {
            Module::Apache => {
                let mut list = vec![(ports.http, "HTTP")];
                if self.ssl_available() {
                    list.push((ports.https, "HTTPS"));
                }
                list
            }
            Module::MySql => vec![(ports.mysql, "MySQL")],
        }
    }

    /// One line on what runs the module, e.g. `Apache 2.4.69 · PHP 8.4.26 (FPM)`.
    pub fn describe(&self, module: Module) -> Option<String> {
        match module {
            Module::Apache => {
                let httpd = self.tools.httpd.as_ref()?;
                let mut text = match &httpd.version {
                    Some(v) => format!("Apache {v}"),
                    None => "Apache".to_string(),
                };
                if let Some(php) = &self.tools.php {
                    text.push_str(" · PHP");
                    if let Some(v) = &php.version {
                        text.push(' ');
                        text.push_str(v);
                    }
                    text.push_str(match php.handler {
                        PhpHandler::Module(_) => " (mod_php)",
                        PhpHandler::Fpm(_) => " (FPM)",
                    });
                }
                Some(text)
            }
            Module::MySql => {
                let db = self.tools.mariadb.as_ref()?;
                Some(match &db.version {
                    Some(v) => format!("MariaDB {v}"),
                    None => "MariaDB".to_string(),
                })
            }
        }
    }

    /// Missing programs; ports taken by someone else (only while stopped).
    pub fn problems(&self, module: Module) -> Vec<Problem> {
        let mut problems = Vec::new();
        match module {
            Module::Apache => match &self.tools.httpd {
                None => problems.push(Problem::Missing {
                    what: "Apache httpd",
                    hint: if cfg!(windows) {
                        "Install gets Apache Lounge's build with winget, \
                         or set the path of httpd.exe in Settings."
                            .into()
                    } else {
                        format!(
                            "Install gets it {}, or set its path in Settings.",
                            install::VIA
                        )
                    },
                }),
                Some(h) if h.modules.is_none() => problems.push(Problem::Missing {
                    what: "Apache's modules directory",
                    hint: "Set the directory holding mod_*.so in Settings.".into(),
                }),
                Some(_) => {
                    if self.tools.php.is_none() {
                        problems.push(Problem::Missing {
                            what: "PHP",
                            hint: if cfg!(windows) {
                                "Install gets PHP's Thread Safe build, which has the Apache \
                                 module (php8apache2_4.dll), with winget, or set the module's \
                                 path in Settings."
                                    .into()
                            } else {
                                format!(
                                    "Install gets it {}, or set the path of PHP-FPM or the \
                                     Apache module (libphp.so) in Settings.",
                                    install::VIA
                                )
                            },
                        });
                    }
                }
            },
            Module::MySql => {
                if self.tools.mariadb.is_none() {
                    problems.push(Problem::Missing {
                        what: "MariaDB",
                        hint: format!(
                            "Install gets it {}, or set the path of {} in Settings.",
                            install::VIA,
                            if cfg!(windows) {
                                "mariadbd.exe"
                            } else {
                                "mariadbd"
                            }
                        ),
                    });
                }
            }
        }
        if self.tracked_alive(module).is_empty() {
            for (port, _) in self.ports(module) {
                if net::port_open(port) {
                    problems.push(Problem::PortInUse {
                        port,
                        owner: net::owner_of(port),
                    });
                }
            }
        }
        problems
    }

    /// The packages the system's package manager can install for what
    /// `module` is missing ([`install::Installer`]).
    pub fn installable(&self, module: Module) -> Vec<Package> {
        Package::ALL
            .into_iter()
            .filter(|p| p.module() == module)
            .filter(|p| match p {
                Package::Apache => self.tools.httpd.is_none(),
                Package::Php => self.tools.php.is_none(),
                Package::MariaDb => self.tools.mariadb.is_none(),
            })
            .collect()
    }

    pub fn status(&self, module: Module) -> ModuleStatus {
        self.reap();
        let pids = self
            .tracked_alive(module)
            .into_iter()
            .map(|t| t.pid)
            .collect();
        let ports = self
            .ports(module)
            .into_iter()
            .map(|(port, label)| PortState {
                port,
                label,
                open: net::port_open(port),
            })
            .collect();
        ModuleStatus {
            module,
            pids,
            ports,
        }
    }

    fn tracked_alive(&self, module: Module) -> Vec<Tracked> {
        process::read_pid_file(&self.layout.pid_file(module))
            .into_iter()
            .filter(Tracked::is_alive)
            .collect()
    }

    /// Writes missing config and starts the module's processes. Returns once
    /// they are launched; use [`Stack::status`] or [`Stack::wait_ready`] to
    /// see them come up. Creating MariaDB's system tables on first start
    /// takes a few seconds.
    pub fn start(&self, module: Module) -> anyhow::Result<Vec<u32>> {
        if !self.tracked_alive(module).is_empty() {
            bail!("{module} is already running");
        }
        if let Some(problem) = self.problems(module).into_iter().next() {
            bail!("{problem}");
        }
        setup::ensure_layout(&self.layout, self.tools.httpd.as_ref())?;
        let procs = match module {
            Module::Apache => self.start_apache()?,
            Module::MySql => self.start_mysql()?,
        };
        process::write_pid_file(&self.layout.pid_file(module), &procs)?;
        Ok(procs.iter().map(|p| p.pid).collect())
    }

    /// Stops the module's processes, killing them if they don't exit in time.
    /// Returns true if they had to be killed.
    pub fn stop(&self, module: Module) -> anyhow::Result<bool> {
        let procs = process::read_pid_file(&self.layout.pid_file(module));
        let grace = match module {
            Module::Apache => Duration::from_secs(10),
            // InnoDB flushes on shutdown; give it time.
            Module::MySql => Duration::from_secs(30),
        };
        let killed = process::terminate(&procs, grace);
        self.reap();
        let _ = std::fs::remove_file(self.layout.pid_file(module));
        if module == Module::Apache {
            let _ = std::fs::remove_file(self.layout.php_fpm_socket());
        }
        Ok(killed)
    }

    /// Blocks until the module accepts connections, or fails with the end of
    /// its logs when it exits or `timeout` passes.
    pub fn wait_ready(&self, module: Module, timeout: Duration) -> anyhow::Result<ModuleStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            let status = self.status(module);
            if status.ready() {
                return Ok(status);
            }
            if !status.running() {
                bail!(
                    "{module} stopped while starting.\n{}",
                    self.failure_details(module)
                );
            }
            if Instant::now() >= deadline {
                bail!(
                    "{module} did not open its ports within {}s.\n{}",
                    timeout.as_secs(),
                    self.failure_details(module)
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// The last lines of the module's logs, to explain a failed start.
    pub fn failure_details(&self, module: Module) -> String {
        let logs = match module {
            Module::Apache => vec![
                self.layout.apache_console_log(),
                self.layout.apache_error_log(),
                self.layout.php_fpm_log(),
            ],
            Module::MySql => vec![
                self.layout.mysql_console_log(),
                self.layout.mysql_error_log(),
            ],
        };
        logs.iter()
            .filter_map(|p| {
                let tail = tail_lines(p, 4)?;
                Some(format!("{}:\n{}", p.display(), tail))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The site, where the Apache row's Admin button goes.
    pub fn site_url(&self) -> String {
        let http = self.settings.ports.http;
        if http == 80 {
            "http://localhost/".to_string()
        } else {
            format!("http://localhost:{http}/")
        }
    }

    /// An admin tool's page, once it is installed.
    pub fn db_admin_url(&self, tool: DbAdmin) -> Option<String> {
        tool.installed(&self.layout)
            .then(|| format!("{}{}/", self.site_url(), tool.id()))
    }

    pub fn config_files(&self, module: Module) -> Vec<NamedFile> {
        let l = &self.layout;
        let mut files = match module {
            Module::Apache => vec![
                named("Apache (httpd.conf)", l.httpd_conf()),
                named("Apache (httpd-ssl.conf)", l.httpd_ssl_conf()),
                named("Apache (httpd-slopp.conf)", l.httpd_slopp_conf()),
                named("PHP (php.ini)", l.php_ini()),
            ],
            Module::MySql => vec![named("MySQL (my.cnf)", l.my_cnf())],
        };
        files.push(named("phpMyAdmin (config.inc.php)", l.phpmyadmin_config()));
        files.push(named("Adminer (index.php)", l.adminer_index()));
        files.push(named("Adminer (adminer.css)", l.adminer_css()));
        files
    }

    pub fn log_files(&self, module: Module) -> Vec<NamedFile> {
        let l = &self.layout;
        match module {
            Module::Apache => vec![
                named("Apache (error.log)", l.apache_error_log()),
                named("Apache (access.log)", l.apache_access_log()),
                named("PHP (php_error_log)", l.php_error_log()),
                named("PHP-FPM (php-fpm.log)", l.php_fpm_log()),
            ],
            Module::MySql => vec![
                named("MySQL (mysql_error.log)", l.mysql_error_log()),
                named("MySQL (install.log)", l.mysql_install_log()),
            ],
        }
    }

    /// Environment for a shell in the stack: server programs first on PATH
    /// and the MariaDB client pointed at this stack's server.
    pub fn shell_env(&self) -> Vec<(String, String)> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut add = |p: Option<&Path>| {
            if let Some(dir) = p.and_then(Path::parent)
                && !dirs.iter().any(|d| d == dir)
            {
                dirs.push(dir.to_path_buf());
            }
        };
        if let Some(db) = &self.tools.mariadb {
            add(db.client.as_deref());
            add(Some(&db.server));
        }
        if let Some(h) = &self.tools.httpd {
            add(Some(&h.bin));
        }
        if let Some(PhpHandler::Fpm(fpm)) = self.tools.php.as_ref().map(|p| &p.handler) {
            add(Some(fpm));
        }
        let path = std::env::var_os("PATH").unwrap_or_default();
        let joined = std::env::join_paths(dirs.into_iter().chain(std::env::split_paths(&path)))
            .unwrap_or(path);
        vec![
            ("PATH".into(), joined.to_string_lossy().into_owned()),
            (
                "MYSQL_UNIX_PORT".into(),
                self.layout.mysql_socket().to_string_lossy().into_owned(),
            ),
            (
                "MYSQL_TCP_PORT".into(),
                self.settings.ports.mysql.to_string(),
            ),
            (
                "SLOPP_ROOT".into(),
                self.layout.root().to_string_lossy().into_owned(),
            ),
        ]
    }

    fn reap(&self) {
        let mut children = self.children.lock().unwrap();
        children.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
    }

    fn keep(&self, child: Child) -> Tracked {
        let tracked = Tracked::of(child.id());
        self.children.lock().unwrap().push(child);
        tracked
    }

    fn start_apache(&self) -> anyhow::Result<Vec<Tracked>> {
        let httpd = self
            .tools
            .httpd
            .as_ref()
            .context("Apache is not installed")?;
        let modules = httpd
            .modules
            .as_ref()
            .context("Apache's modules directory was not found")?;
        let php = self.tools.php.as_ref().context("PHP is not installed")?;
        let layout = &self.layout;
        let ports = self.settings.ports;

        let mut ssl = self.ssl_available();
        if ssl && let Err(err) = setup::ensure_certificate(layout) {
            tracing::warn!("HTTPS disabled: {err:#}");
            ssl = false;
        }

        let mut args: Vec<String> = vec![
            "-f".into(),
            conf_path(&layout.httpd_conf()),
            "-C".into(),
            format!("Define SLOPP_MODULES \"{}\"", conf_path(modules)),
            "-C".into(),
            format!("Define HTTP_PORT {}", ports.http),
            "-C".into(),
            format!("Define HTTPS_PORT {}", ports.https),
        ];
        if ssl {
            args.extend(["-D".into(), "SSL".into()]);
        }

        let mut procs = Vec::new();
        let mut env = Vec::new();
        match &php.handler {
            PhpHandler::Module(lib) => {
                args.extend([
                    "-D".into(),
                    "PHP_MODULE".into(),
                    "-C".into(),
                    format!("Define PHP_MODULE_PATH \"{}\"", conf_path(lib)),
                ]);
                if cfg!(windows) {
                    if let Some(dir) = lib.parent() {
                        env = self.windows_php_env(dir)?;
                    }
                } else {
                    let cli = crate::toolchain::find_program(&["php"]);
                    env = self.unix_php_env(cli.as_deref())?;
                }
            }
            PhpHandler::Fpm(fpm) => {
                args.extend([
                    "-D".into(),
                    "PHP_FPM".into(),
                    "-C".into(),
                    format!(
                        "Define PHP_FPM_SOCKET \"{}\"",
                        conf_path(&layout.php_fpm_socket())
                    ),
                ]);
                let fpm_env = self.unix_php_env(Some(fpm))?;
                procs.push(self.start_php_fpm(fpm, fpm_env)?);
            }
        }

        // Check the config first: a typo is reported in a second, with the
        // line number, instead of as a vanished process.
        let check = process::command(&httpd.bin)
            .args(&args)
            .arg("-t")
            .envs(env.iter().cloned())
            .output()
            .with_context(|| format!("running {}", httpd.bin.display()))?;
        if !check.status.success() {
            process::terminate(&procs, Duration::from_secs(5));
            let mut text = process::decode(&check.stderr);
            text.push_str(&process::decode(&check.stdout));
            bail!("Apache's configuration has an error:\n{}", text.trim());
        }

        let mut cmd = Command::new(&httpd.bin);
        cmd.args(&args).arg("-DFOREGROUND").envs(env);
        match process::spawn_detached(cmd, &layout.apache_console_log()) {
            Ok(child) => procs.insert(0, self.keep(child)),
            Err(err) => {
                process::terminate(&procs, Duration::from_secs(5));
                return Err(err);
            }
        }
        Ok(procs)
    }

    /// What mod_php needs on Windows from Apache's environment: PHP's
    /// folder on `PATH`, since the libraries of PHP and its extensions
    /// (OpenSSL, ICU, ...) sit there and Windows looks for them only on
    /// `PATH`; and a `PHP_INI_SCAN_DIR` whose `slopp.ini` sets
    /// `extension_dir` to this PHP's `ext`, for the extensions php.ini names,
    /// and mysqli's default port.
    fn windows_php_env(&self, php_dir: &Path) -> anyhow::Result<Vec<(String, std::ffi::OsString)>> {
        let layout = &self.layout;
        let scan_dir = layout.php_scan_dir();
        std::fs::create_dir_all(&scan_dir)
            .with_context(|| format!("creating {}", scan_dir.display()))?;
        let ini = scan_dir.join("slopp.ini");
        let ini_text = templates::php_scan_ini(&php_dir.join("ext"), self.settings.ports.mysql);
        std::fs::write(&ini, ini_text).with_context(|| format!("writing {}", ini.display()))?;

        let path = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(php_dir.to_path_buf()).chain(std::env::split_paths(&path)),
        )
        .unwrap_or(path);
        Ok(vec![
            ("PATH".into(), path),
            ("PHP_INI_SCAN_DIR".into(), scan_dir.into_os_string()),
        ])
    }

    /// What PHP needs from its environment on Linux and macOS: when `php`
    /// (the CLI beside the Apache module, or PHP-FPM) has extensions that
    /// SLOPP's php.ini wants but doesn't load them by itself (Arch enables
    /// mysqli in its own php.ini, which SLOPP's replaces), a `slopp.ini`
    /// that loads them, in a `PHP_INI_SCAN_DIR` read after PHP's own (the
    /// leading `:` keeps that). Nothing otherwise.
    fn unix_php_env(
        &self,
        php: Option<&Path>,
    ) -> anyhow::Result<Vec<(String, std::ffi::OsString)>> {
        let layout = &self.layout;
        let scan_dir = layout.php_scan_dir();
        let ini = scan_dir.join("slopp.ini");
        let unloaded = php
            .map(|php| crate::toolchain::unloaded_extensions(php, &layout.php_ini()))
            .unwrap_or_default();
        if unloaded.is_empty() {
            let _ = std::fs::remove_file(&ini);
            return Ok(Vec::new());
        }
        tracing::info!("loading PHP extensions {unloaded:?} from {}", ini.display());
        std::fs::create_dir_all(&scan_dir)
            .with_context(|| format!("creating {}", scan_dir.display()))?;
        std::fs::write(&ini, templates::php_extensions_ini(&unloaded))
            .with_context(|| format!("writing {}", ini.display()))?;
        let mut value = std::ffi::OsString::from(":");
        value.push(scan_dir.as_os_str());
        Ok(vec![("PHP_INI_SCAN_DIR".into(), value)])
    }

    fn start_php_fpm(
        &self,
        fpm: &Path,
        env: Vec<(String, std::ffi::OsString)>,
    ) -> anyhow::Result<Tracked> {
        let layout = &self.layout;
        std::fs::write(layout.php_fpm_conf(), templates::php_fpm_conf(layout))
            .with_context(|| format!("writing {}", layout.php_fpm_conf().display()))?;
        let socket = layout.php_fpm_socket();
        let _ = std::fs::remove_file(&socket);

        let mut cmd = Command::new(fpm);
        cmd.arg("--nodaemonize")
            .arg("--fpm-config")
            .arg(layout.php_fpm_conf())
            .arg("-c")
            .arg(layout.php_ini())
            .envs(env);
        let child = process::spawn_detached(cmd, &layout.php_fpm_log())?;
        let tracked = self.keep(child);

        let deadline = Instant::now() + Duration::from_secs(5);
        while !socket.exists() {
            if !tracked.is_alive() {
                bail!(
                    "PHP-FPM stopped while starting:\n{}",
                    tail_lines(&layout.php_fpm_log(), 6).unwrap_or_default()
                );
            }
            if Instant::now() >= deadline {
                process::terminate(&[tracked], Duration::from_secs(2));
                bail!("PHP-FPM did not open its socket within 5s");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(tracked)
    }

    fn start_mysql(&self) -> anyhow::Result<Vec<Tracked>> {
        let db = self
            .tools
            .mariadb
            .as_ref()
            .context("MariaDB is not installed")?;
        let layout = &self.layout;
        setup::init_mysql(layout, db)?;
        let mut cmd = Command::new(&db.server);
        // --defaults-file must come first.
        cmd.arg(format!("--defaults-file={}", layout.my_cnf().display()))
            .arg(format!("--port={}", self.settings.ports.mysql));
        // MariaDB before 10.11 reads my.cnf in the Windows code page, not
        // UTF-8, so a path with "á" in it there would point elsewhere.
        // Windows hands arguments over in whichever the program uses.
        if cfg!(windows) {
            cmd.arg(format!("--datadir={}", layout.mysql_data().display()))
                .arg(format!("--tmpdir={}", layout.tmp().display()))
                .arg(format!(
                    "--log-error={}",
                    layout.mysql_error_log().display()
                ))
                .arg(format!("--pid-file={}", layout.mysql_pid_file().display()));
        }
        let child = process::spawn_detached(cmd, &layout.mysql_console_log())?;
        Ok(vec![self.keep(child)])
    }
}

/// The last `n` non-empty lines of a file.
pub fn tail_lines(path: &Path, n: usize) -> Option<String> {
    let text = process::decode(&std::fs::read(path).ok()?);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return None;
    }
    Some(lines[lines.len().saturating_sub(n)..].join("\n"))
}

/// Starts every module in `modules`, stopping at the first failure.
pub fn start_all(stack: &Stack, modules: &[Module], timeout: Duration) -> anyhow::Result<()> {
    for &module in modules {
        stack
            .start(module)
            .and_then(|_| stack.wait_ready(module, timeout))
            .map_err(|err| anyhow!("{module}: {err:#}"))?;
    }
    Ok(())
}
