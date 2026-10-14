//! `sloppctl`: the stack from a terminal, like XAMPP's `lampp` script.

use std::process::ExitCode;
use std::time::Duration;

use slopp_core::dbadmin::Progress;
use slopp_core::{DbAdmin, Module, Settings, Stack, net, winget};

const USAGE: &str = "\
Usage: sloppctl <command> [apache|mysql]

Commands:
  start [module]       Start Apache and MySQL, or one of them
  stop [module]        Stop them
  restart [module]     Stop, then start
  status               Show what's running
  ports                List programs listening on TCP ports
  paths                Show where the stack and its programs are
  install-phpmyadmin   Download phpMyAdmin into the stack
  install-adminer      Download Adminer into the stack
  install-servers [module]
                       Install missing servers with winget (Windows)

The stack lives in $SLOPP_ROOT, or the root set in the panel's settings.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let modules = match args.get(1) {
        None => Module::ALL.to_vec(),
        Some(id) => match Module::from_id(id) {
            Some(m) => vec![m],
            None => {
                eprintln!("Unknown module '{id}'. Use apache or mysql.");
                return ExitCode::from(2);
            }
        },
    };

    let settings = match Settings::load(&Settings::default_path()) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("{err:#}");
            return ExitCode::FAILURE;
        }
    };
    let stack = Stack::new(settings);

    let ok = match command.as_str() {
        "start" => start(&stack, &modules),
        "stop" => stop(&stack, &modules),
        "restart" => stop(&stack, &modules) && start(&stack, &modules),
        "status" => {
            status(&stack);
            true
        }
        "ports" => {
            ports();
            true
        }
        "paths" => {
            paths(&stack);
            true
        }
        "install-phpmyadmin" => install_db_admin(&stack, DbAdmin::PhpMyAdmin),
        "install-adminer" => install_db_admin(&stack, DbAdmin::Adminer),
        "install-servers" => install_servers(&stack, &modules),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            true
        }
        other => {
            eprintln!("Unknown command '{other}'.\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn start(stack: &Stack, modules: &[Module]) -> bool {
    let mut ok = true;
    for &m in modules {
        if stack.status(m).running() {
            println!("{m} is already running.");
            continue;
        }
        println!("Starting {m}...");
        match stack
            .start(m)
            .and_then(|_| stack.wait_ready(m, Duration::from_secs(30)))
        {
            Ok(status) => {
                let ports: Vec<String> = status.ports.iter().map(|p| p.port.to_string()).collect();
                println!("{m} is running on port {}.", ports.join(", "));
            }
            Err(err) => {
                eprintln!("{m} didn't start: {err:#}");
                ok = false;
            }
        }
    }
    ok
}

fn stop(stack: &Stack, modules: &[Module]) -> bool {
    let mut ok = true;
    for &m in modules {
        if !stack.status(m).running() {
            println!("{m} is not running.");
            continue;
        }
        println!("Stopping {m}...");
        match stack.stop(m) {
            Ok(true) => println!("{m} didn't stop in time and was killed."),
            Ok(false) => println!("{m} stopped."),
            Err(err) => {
                eprintln!("Stopping {m} failed: {err:#}");
                ok = false;
            }
        }
    }
    ok
}

fn status(stack: &Stack) {
    for m in Module::ALL {
        let s = stack.status(m);
        let ports: Vec<String> = s
            .ports
            .iter()
            .map(|p| {
                format!(
                    "{} {}{}",
                    p.label,
                    p.port,
                    if p.open { "" } else { " (closed)" }
                )
            })
            .collect();
        if s.running() {
            let pids: Vec<String> = s.pids.iter().map(u32::to_string).collect();
            println!(
                "{:<7} running  pid {}  {}",
                m.name(),
                pids.join(","),
                ports.join(", ")
            );
        } else {
            println!("{:<7} stopped", m.name());
            for problem in stack.problems(m) {
                println!("        {problem}");
            }
        }
    }
}

fn ports() {
    println!("{:<8} {:<40} {:<8} PROGRAM", "PORT", "ADDRESS", "PID");
    for l in net::listeners() {
        println!(
            "{:<8} {:<40} {:<8} {}",
            l.port(),
            l.addr.ip(),
            l.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
            l.process.as_deref().unwrap_or("-"),
        );
    }
}

fn paths(stack: &Stack) {
    let tools = stack.tools();
    let show = |p: Option<&std::path::Path>| {
        p.map(|p| p.display().to_string())
            .unwrap_or_else(|| "not found".into())
    };
    println!("Stack root    {}", stack.layout().root().display());
    println!("Settings      {}", Settings::default_path().display());
    println!(
        "httpd         {}",
        show(tools.httpd.as_ref().map(|h| h.bin.as_path()))
    );
    println!(
        "Apache mods   {}",
        show(tools.httpd.as_ref().and_then(|h| h.modules.as_deref()))
    );
    let php = tools.php.as_ref().map(|p| match &p.handler {
        slopp_core::toolchain::PhpHandler::Module(lib) => format!("{} (mod_php)", lib.display()),
        slopp_core::toolchain::PhpHandler::Fpm(fpm) => format!("{} (FPM)", fpm.display()),
    });
    println!(
        "PHP           {}",
        php.unwrap_or_else(|| "not found".into())
    );
    println!(
        "mariadbd      {}",
        show(tools.mariadb.as_ref().map(|m| m.server.as_path()))
    );
}

fn install_servers(stack: &Stack, modules: &[Module]) -> bool {
    if !cfg!(windows) {
        eprintln!("Install Apache, PHP and MariaDB with your system's package manager.");
        return false;
    }
    let packages: Vec<_> = modules.iter().flat_map(|&m| stack.installable(m)).collect();
    if packages.is_empty() {
        println!("Nothing to install: the servers were found.");
        return true;
    }
    let Some(winget) = winget::find() else {
        eprintln!(
            "Installing the servers needs winget, which comes with App Installer \
             from the Microsoft Store: {}",
            winget::GET_WINGET_URL
        );
        return false;
    };
    for package in packages {
        println!(
            "Installing {} ({}) with winget...",
            package.name(),
            package.id()
        );
        if let Err(err) = winget::install(&winget, package, |line| println!("  {line}")) {
            eprintln!("Installing {} failed: {err:#}", package.name());
            return false;
        }
    }
    // A new process finds what was installed.
    let stack = Stack::new(stack.settings().clone());
    for &m in modules {
        match stack.describe(m) {
            Some(text) => println!("{m}: {text}"),
            None => println!("{m}: still not found; see `sloppctl status`."),
        }
    }
    true
}

fn install_db_admin(stack: &Stack, tool: DbAdmin) -> bool {
    let name = tool.name();
    let mut last = String::new();
    let result = tool.install(stack.layout(), stack.settings().ports.mysql, |p| {
        let line = match p {
            Progress::Checking => format!("Looking up the latest {name}..."),
            Progress::Downloading {
                version,
                fraction: Some(f),
            } => format!("Downloading {name} {version}... {:.0}%", f * 100.0),
            Progress::Downloading { version, .. } => format!("Downloading {name} {version}..."),
            Progress::Unpacking { version } => format!("Unpacking {name} {version}..."),
        };
        if line != last {
            println!("{line}");
            last = line;
        }
    });
    match result {
        Ok(installed) => {
            println!("{name} {} is installed.", installed.version);
            if installed.apache_conf_changed && stack.status(Module::Apache).running() {
                println!("Restart Apache to serve it: sloppctl restart apache");
            }
            true
        }
        Err(err) => {
            eprintln!("Installing {name} failed: {err:#}");
            false
        }
    }
}
