//! Starting server processes so they outlive the panel, remembering them in
//! pid files, and telling whether they are still the processes we started.

use std::fs::OpenOptions;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context as _;

/// A process we started. `started` tells a reused pid apart from ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tracked {
    pub pid: u32,
    /// Platform start-time token (Linux: clock ticks since boot), if known.
    pub started: Option<u64>,
}

impl Tracked {
    pub fn of(pid: u32) -> Self {
        Self {
            pid,
            started: start_time(pid),
        }
    }

    /// Whether this is still the process we started, and not a zombie.
    pub fn is_alive(&self) -> bool {
        if !pid_alive(self.pid) {
            return false;
        }
        match (self.started, start_time(self.pid)) {
            (Some(ours), Some(now)) => ours == now,
            _ => true,
        }
    }
}

/// Starts `cmd` in its own process group with output appended to `log`, so
/// it keeps running when the panel exits and doesn't get the terminal's ^C.
pub fn spawn_detached(mut cmd: Command, log: &Path) -> anyhow::Result<Child> {
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let out = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .with_context(|| format!("opening {}", log.display()))?;
    let err = out.try_clone()?;
    cmd.stdin(Stdio::null()).stdout(out).stderr(err);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    let program = cmd.get_program().to_string_lossy().into_owned();
    cmd.spawn().with_context(|| format!("starting {program}"))
}

/// Reads `pid starttime` lines. A missing file is no processes.
pub fn read_pid_file(path: &Path) -> Vec<Tracked> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse().ok()?;
            let started = parts.next().and_then(|s| s.parse().ok());
            Some(Tracked { pid, started })
        })
        .collect()
}

pub fn write_pid_file(path: &Path, procs: &[Tracked]) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text: String = procs
        .iter()
        .map(|p| match p.started {
            Some(s) => format!("{} {}\n", p.pid, s),
            None => format!("{}\n", p.pid),
        })
        .collect();
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

/// Asks processes to exit, waits up to `grace`, then kills what's left.
/// Returns true if anything had to be killed.
pub fn terminate(procs: &[Tracked], grace: Duration) -> bool {
    let alive: Vec<_> = procs.iter().filter(|p| p.is_alive()).copied().collect();
    for p in &alive {
        signal_term(p.pid);
    }
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if alive.iter().all(|p| !p.is_alive()) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut killed = false;
    for p in alive.iter().filter(|p| p.is_alive()) {
        signal_kill(p.pid);
        killed = true;
    }
    killed
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    let exists = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0
        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    exists && !is_zombie(pid)
}

#[cfg(target_os = "linux")]
fn proc_stat(pid: u32) -> Option<Vec<String>> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The name (field 2) is in parentheses and may contain spaces.
    let after_name = &stat[stat.rfind(')')? + 1..];
    Some(after_name.split_whitespace().map(str::to_string).collect())
}

#[cfg(target_os = "linux")]
fn is_zombie(pid: u32) -> bool {
    // Field 3 is the state; it is the first field after the name.
    proc_stat(pid).is_some_and(|f| f.first().is_some_and(|s| s == "Z" || s == "X"))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn is_zombie(_pid: u32) -> bool {
    false
}

/// Linux: field 22 of `/proc/<pid>/stat`.
#[cfg(target_os = "linux")]
fn start_time(pid: u32) -> Option<u64> {
    // Fields after the name start at field 3, so field 22 is index 19.
    proc_stat(pid)?.get(19)?.parse().ok()
}

#[cfg(not(target_os = "linux"))]
fn start_time(_pid: u32) -> Option<u64> {
    None
}

#[cfg(unix)]
fn signal_term(pid: u32) {
    // SAFETY: plain kill(2).
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
}

#[cfg(unix)]
fn signal_kill(pid: u32) {
    // SAFETY: plain kill(2).
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(windows)]
fn pid_alive(pid: u32) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")))
}

#[cfg(windows)]
fn signal_term(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T"])
        .output();
}

#[cfg(windows)]
fn signal_kill(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .output();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_file_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run/x.pids");
        let procs = [
            Tracked {
                pid: 10,
                started: Some(99),
            },
            Tracked {
                pid: 11,
                started: None,
            },
        ];
        write_pid_file(&path, &procs).unwrap();
        assert_eq!(read_pid_file(&path), procs);
        assert!(read_pid_file(&dir.path().join("missing")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn tracks_and_terminates_a_detached_process() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let mut child = spawn_detached(cmd, &dir.path().join("log")).unwrap();
        let tracked = Tracked::of(child.id());
        assert!(tracked.is_alive());

        // A different start time means the pid was reused by someone else.
        if let Some(started) = tracked.started {
            let stale = Tracked {
                pid: tracked.pid,
                started: Some(started + 1),
            };
            assert!(!stale.is_alive());
        }

        // Reap in the background so the child doesn't linger as a zombie.
        let reaper = std::thread::spawn(move || child.wait());
        assert!(!terminate(&[tracked], Duration::from_secs(5)));
        reaper.join().unwrap().unwrap();
        assert!(!tracked.is_alive());
    }
}
