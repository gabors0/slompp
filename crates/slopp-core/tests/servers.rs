//! Starts the real servers (Apache with PHP, MariaDB, Mailpit) in a stack
//! root of their own, on free ports, and checks what `Stack` promises about
//! them.
//!
//! They need Apache, PHP (PHP-FPM or mod_php, with mysqli), MariaDB and
//! Mailpit installed, and a regular user (mariadbd and php-fpm refuse to run as
//! root), so they only run when asked:
//!
//! ```sh
//! cargo test -p slopp-core --test servers -- --ignored
//! ```

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use slopp_core::stack::Problem;
use slopp_core::{Module, Ports, Settings, Stack, setup};

/// One test at a time: they pick free ports, and each start of MariaDB
/// takes a few seconds of disk and CPU.
static SERIAL: Mutex<()> = Mutex::new(());

const READY: Duration = Duration::from_secs(60);

/// A stack in a temporary root, stopped when dropped so a failed assertion
/// doesn't leave detached servers behind.
struct Fixture {
    stack: Stack,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        #[cfg(unix)]
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            panic!("run these as a regular user: mariadbd and php-fpm refuse to run as root");
        }
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            root: Some(dir.path().join("root")),
            ports: free_ports(),
            ..Settings::default()
        };
        let stack = Stack::new(settings);
        let tools = stack.tools();
        let missing: Vec<&str> = [
            ("Apache", tools.httpd.is_none()),
            ("PHP", tools.php.is_none()),
            ("MariaDB", tools.mariadb.is_none()),
            ("Mailpit", tools.mailpit.is_none()),
        ]
        .into_iter()
        .filter_map(|(name, missing)| missing.then_some(name))
        .collect();
        assert!(missing.is_empty(), "not installed: {missing:?}");
        Fixture { stack, _dir: dir }
    }

    fn start(&self, module: Module) {
        let pids = self
            .stack
            .start(module)
            .unwrap_or_else(|err| panic!("starting {module}: {err:#}"));
        assert!(!pids.is_empty());
        let status = self
            .stack
            .wait_ready(module, READY)
            .unwrap_or_else(|err| panic!("{module} didn't come up: {err:#}"));
        assert_eq!(status.pids.len(), pids.len(), "{module}: {status:?}");
    }

    /// Stops `module`, which must exit on its own (but for Mailpit on
    /// Windows), and checks nothing of it is left.
    fn stop(&self, module: Module) {
        let layout = self.stack.layout();
        let killed = self.stack.stop(module).unwrap();
        // Windows has no SIGTERM, and Mailpit, unlike Apache and MariaDB,
        // has no shutdown event to set, so process::terminate kills it.
        if !(cfg!(windows) && module == Module::Mailpit) {
            assert!(!killed, "{module} had to be killed");
        }
        let status = self.stack.status(module);
        assert!(!status.running(), "{status:?}");
        assert!(!layout.pid_file(module).exists());
        for (port, _) in self.stack.ports(module) {
            assert!(eventually_closed(port), "{module} still listens on {port}");
        }
        if module == Module::Apache {
            assert!(!layout.php_fpm_socket().exists());
        }
    }

    fn page(&self, name: &str, php: &str) {
        std::fs::write(self.stack.layout().htdocs().join(name), php).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for module in Module::ALL {
            // The root goes with the test, so show why it failed first.
            let logs = self.stack.failure_details(module);
            if std::thread::panicking() && !logs.is_empty() {
                eprintln!("--- {module}'s logs\n{logs}");
            }
            let _ = self.stack.stop(module);
        }
    }
}

/// Ports nothing listens on right now.
fn free_ports() -> Ports {
    let listeners: Vec<TcpListener> = (0..5)
        .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
        .collect();
    let port = |i: usize| listeners[i].local_addr().unwrap().port();
    Ports {
        http: port(0),
        https: port(1),
        mysql: port(2),
        smtp: port(3),
        mailpit: port(4),
    }
}

/// Sends a message over SMTP, checking each reply's code.
fn send_mail(port: u16, subject: &str) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let mut replies = BufReader::new(stream.try_clone().unwrap());
    // A reply's last line has a space after the code, the others a dash.
    let mut expect = |code: &str| loop {
        let mut line = String::new();
        replies.read_line(&mut line).unwrap();
        assert!(line.starts_with(code), "wanted {code}, got {line:?}");
        if line.as_bytes().get(3) != Some(&b'-') {
            break;
        }
    };
    expect("220");
    for (command, code) in [
        ("EHLO localhost", "250"),
        ("MAIL FROM:<app@example.com>", "250"),
        ("RCPT TO:<someone@example.com>", "250"),
        ("DATA", "354"),
    ] {
        write!(stream, "{command}\r\n").unwrap();
        expect(code);
    }
    write!(
        stream,
        "From: app@example.com\r\nTo: someone@example.com\r\nSubject: {subject}\r\n\r\nHello.\r\n.\r\n"
    )
    .unwrap();
    expect("250");
    write!(stream, "QUIT\r\n").unwrap();
    expect("221");
}

fn eventually_closed(port: u16) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while slopp_core::net::port_open(port) {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    true
}

/// A plain HTTP/1.0 GET (no chunked bodies): the status code and the body.
fn get(port: u16, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    write!(stream, "GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("not an HTTP response: {response:?}"));
    let code = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("no status in {head:?}"));
    (code, body.to_string())
}

fn locked() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
#[ignore = "starts Apache and PHP"]
fn apache_serves_php_and_stops_cleanly() {
    let _serial = locked();
    let fx = Fixture::new();
    let stack = &fx.stack;
    assert!(!stack.status(Module::Apache).running());
    assert_eq!(stack.problems(Module::Apache), vec![]);

    fx.start(Module::Apache);
    let status = stack.status(Module::Apache);
    let expected = if stack.ssl_available() { 2 } else { 1 };
    assert_eq!(status.ports.len(), expected, "{status:?}");
    assert!(status.ready(), "{status:?}");

    let http = stack.settings().ports.http;
    let (code, body) = get(http, "/");
    assert_eq!(code, 200, "{body}");
    assert!(body.contains("SLOPP"), "{body}");
    fx.page("v.php", "<?php echo 'php ', PHP_MAJOR_VERSION;");
    assert_eq!(
        get(http, "/v.php"),
        (200, format!("php {}", php_major(&fx)))
    );

    let again = stack.start(Module::Apache).unwrap_err().to_string();
    assert!(again.contains("already running"), "{again}");

    fx.stop(Module::Apache);
    // A second stop has nothing to do.
    assert!(!stack.stop(Module::Apache).unwrap());
}

/// PHP's major version as the toolchain saw it.
fn php_major(fx: &Fixture) -> String {
    let php = fx.stack.tools().php.as_ref().unwrap();
    let version = php.version.as_deref().expect("PHP's version");
    version.split('.').next().unwrap().to_string()
}

#[test]
#[ignore = "starts MariaDB, Apache and PHP"]
fn php_reaches_mariadb_and_mariadb_restarts() {
    let _serial = locked();
    let fx = Fixture::new();
    let stack = &fx.stack;

    // The first start creates the system tables.
    fx.start(Module::MySql);
    assert!(setup::mysql_initialised(stack.layout()));
    fx.start(Module::Apache);

    // As root without a password, the way XAMPP ships and phpMyAdmin logs
    // in; "localhost" is the socket (or the port, on Windows) php.ini names.
    fx.page(
        "db.php",
        "<?php $db = new mysqli('localhost', 'root', '');\n\
         echo $db->query('SELECT 6 * 7')->fetch_row()[0];",
    );
    let http = stack.settings().ports.http;
    assert_eq!(get(http, "/db.php"), (200, "42".to_string()));

    // InnoDB shuts down cleanly, and the next start reuses the data.
    fx.stop(Module::MySql);
    assert_ne!(get(http, "/db.php").1, "42");
    fx.start(Module::MySql);
    assert_eq!(get(http, "/db.php"), (200, "42".to_string()));

    fx.stop(Module::Apache);
    fx.stop(Module::MySql);
}

#[test]
#[ignore = "starts Apache and PHP"]
fn apache_config_errors_are_reported_and_nothing_is_left_running() {
    let _serial = locked();
    let fx = Fixture::new();
    let stack = &fx.stack;
    let layout = stack.layout();
    setup::ensure_layout(layout, stack.tools().httpd.as_ref()).unwrap();
    let mut conf = std::fs::read_to_string(layout.httpd_conf()).unwrap();
    conf.push_str("\nNotADirective here\n");
    std::fs::write(layout.httpd_conf(), conf).unwrap();

    let err = format!("{:#}", stack.start(Module::Apache).unwrap_err());
    assert!(err.contains("configuration has an error"), "{err}");
    assert!(err.contains("NotADirective"), "{err}");
    assert!(err.contains("httpd.conf"), "{err}");

    // PHP-FPM, started before the check, was stopped again.
    assert!(!stack.status(Module::Apache).running());
    assert!(!layout.pid_file(Module::Apache).exists());
    let deadline = Instant::now() + Duration::from_secs(5);
    while layout.php_fpm_socket().exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!layout.php_fpm_socket().exists(), "PHP-FPM is still up");
}

#[test]
#[ignore = "needs Apache and PHP installed"]
fn a_taken_port_is_a_problem_and_blocks_the_start() {
    let _serial = locked();
    let fx = Fixture::new();
    let stack = &fx.stack;
    let http = stack.settings().ports.http;
    let _taken = TcpListener::bind(("127.0.0.1", http)).unwrap();

    let problems = stack.problems(Module::Apache);
    let Some(Problem::PortInUse { port, owner }) = problems.first() else {
        panic!("{problems:?}");
    };
    assert_eq!(*port, http);
    // Our own socket, so its owner is visible everywhere.
    let owner = owner.as_ref().expect("an owner");
    assert_eq!(owner.pid, Some(std::process::id()), "{owner:?}");

    let err = stack.start(Module::Apache).unwrap_err().to_string();
    assert!(err.contains(&format!("Port {http} is in use")), "{err}");
    assert!(!stack.status(Module::Apache).running());
}

#[test]
#[ignore = "starts MariaDB, Apache and PHP"]
fn new_ports_need_no_config_changes() {
    let _serial = locked();
    let mut fx = Fixture::new();
    fx.start(Module::Apache);
    fx.start(Module::MySql);
    fx.stop(Module::Apache);
    fx.stop(Module::MySql);

    let layout = fx.stack.layout().clone();
    let configs = [layout.httpd_conf(), layout.my_cnf(), layout.php_ini()];
    let read = || {
        configs
            .iter()
            .map(|p| std::fs::read(p).unwrap())
            .collect::<Vec<_>>()
    };
    let before = read();

    let old = fx.stack.settings().ports;
    let mut settings = fx.stack.settings().clone();
    // Free now, and none of the old ones, which are free again too.
    settings.ports = loop {
        let ports = free_ports();
        let list = [ports.http, ports.https, ports.mysql];
        if ![old.http, old.https, old.mysql]
            .iter()
            .any(|p| list.contains(p))
        {
            break ports;
        }
    };
    fx.stack = fx.stack.reconfigure(settings);
    let new = fx.stack.settings().ports;

    fx.start(Module::Apache);
    fx.start(Module::MySql);
    assert_eq!(get(new.http, "/").0, 200);
    assert!(slopp_core::net::port_open(new.mysql));
    for port in [old.http, old.https, old.mysql] {
        assert!(!slopp_core::net::port_open(port), "{port} is open");
    }
    assert!(read() == before, "a config file changed");

    fx.stop(Module::Apache);
    fx.stop(Module::MySql);
}

#[test]
#[ignore = "starts Mailpit"]
fn mailpit_catches_mail_and_keeps_it() {
    let _serial = locked();
    let fx = Fixture::new();
    let stack = &fx.stack;
    let ports = stack.settings().ports;
    assert_eq!(stack.problems(Module::Mailpit), vec![]);
    let describe = stack.describe(Module::Mailpit).unwrap();
    assert!(describe.starts_with("Mailpit 1."), "{describe}");

    fx.start(Module::Mailpit);
    send_mail(ports.smtp, "Caught by SLOPP");
    let (code, body) = get(ports.mailpit, "/api/v1/messages");
    assert_eq!(code, 200, "{body}");
    assert!(body.contains("Caught by SLOPP"), "{body}");
    assert!(stack.layout().mailpit_db().exists());
    fx.stop(Module::Mailpit);

    // The mail is in the stack, not in Mailpit's memory.
    fx.start(Module::Mailpit);
    let (_, body) = get(ports.mailpit, "/api/v1/messages");
    assert!(body.contains("Caught by SLOPP"), "{body}");
    fx.stop(Module::Mailpit);
}

/// Through run/sendmail on Linux and macOS, over SMTP on Windows, where PHP
/// also needs php.ini's `sendmail_from` for a message without `From:`.
#[test]
#[ignore = "starts Mailpit, Apache and PHP"]
fn php_mail_goes_to_mailpit() {
    let _serial = locked();
    let fx = Fixture::new();
    let ports = fx.stack.settings().ports;
    fx.start(Module::Mailpit);
    fx.start(Module::Apache);
    fx.page(
        "mail.php",
        "<?php var_export(mail('someone@example.com', 'Sent by PHP', 'Hello.'));",
    );
    let (code, body) = get(ports.http, "/mail.php");
    assert_eq!((code, body.as_str()), (200, "true"));
    let (_, body) = get(ports.mailpit, "/api/v1/messages");
    assert!(body.contains("Sent by PHP"), "{body}");
    fx.stop(Module::Apache);
    fx.stop(Module::Mailpit);
}
