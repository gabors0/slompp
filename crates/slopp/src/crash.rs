//! Keeps the message of a panic. A release build on Windows has no console,
//! so it would go nowhere: the hook writes it to `crash.log` beside the
//! settings, and the next start reports it in the log.

use std::path::{Path, PathBuf};

use slopp_core::Settings;

pub fn log_path() -> PathBuf {
    Settings::default_path().with_file_name("crash.log")
}

/// Writes every panic to `path` before the default hook runs.
pub fn install(path: PathBuf) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let text = format!(
            "SLOPP {} on thread '{}' {info}\n\n{}\n",
            env!("CARGO_PKG_VERSION"),
            thread.name().unwrap_or("unnamed"),
            std::backtrace::Backtrace::force_capture(),
        );
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, text);
        default(info);
    }));
}

/// What the last run's panic said, once: the log is renamed to
/// `crash.log.old` so the next start doesn't report it again.
pub fn take_last(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let _ = std::fs::rename(path, path.with_extension("log.old"));
    Some(summary(&text))
}

/// The header and the panic message, without the backtrace.
fn summary(text: &str) -> String {
    text.split("\n\n")
        .next()
        .unwrap_or(text)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarises_without_the_backtrace() {
        let text = "SLOPP 0.2.1 on thread 'main' panicked at crates/slopp/src/x.rs:3:5:\n\
                    index out of bounds\n\n   0: std::backtrace\n   1: slopp::main\n";
        assert_eq!(
            summary(text),
            "SLOPP 0.2.1 on thread 'main' panicked at crates/slopp/src/x.rs:3:5: index out of bounds"
        );
    }

    #[test]
    fn reports_a_crash_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crash.log");
        assert_eq!(take_last(&path), None);
        std::fs::write(
            &path,
            "SLOPP 0.2.1 on thread 'main' panicked at a.rs:1:1:\nboom\n\nbt",
        )
        .unwrap();
        assert_eq!(
            take_last(&path).as_deref(),
            Some("SLOPP 0.2.1 on thread 'main' panicked at a.rs:1:1: boom")
        );
        assert_eq!(take_last(&path), None);
        assert!(dir.path().join("crash.log.old").is_file());
    }
}
