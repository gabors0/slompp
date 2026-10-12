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
    /// Platform start-time token (Linux: clock ticks since boot; macOS:
    /// microseconds since the epoch), if known.
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
        // Blocked and ignored signals stay that way across exec, and the
        // panel starts servers from threads that block most of them (GPUI's
        // background executor on macOS is Grand Central Dispatch). A server
        // that never sees SIGTERM can't be stopped: mod_php's Apache workers
        // outlive their parent and keep the ports.
        // SAFETY: sigprocmask(2) and signal(2) are async-signal-safe.
        unsafe {
            cmd.pre_exec(|| {
                let mut none = std::mem::zeroed::<libc::sigset_t>();
                libc::sigemptyset(&mut none);
                libc::sigprocmask(libc::SIG_SETMASK, &none, std::ptr::null_mut());
                for signal in DEFAULTED_SIGNALS {
                    libc::signal(*signal, libc::SIG_DFL);
                }
                Ok(())
            });
        }
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

/// Signals a server starts with the default action for, whatever we ignore.
#[cfg(unix)]
const DEFAULTED_SIGNALS: &[libc::c_int] = &[
    libc::SIGHUP,
    libc::SIGINT,
    libc::SIGQUIT,
    libc::SIGTERM,
    libc::SIGUSR1,
    libc::SIGUSR2,
    libc::SIGALRM,
    libc::SIGCHLD,
];

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

/// `proc_pidinfo`'s summary of a process, or the errno it failed with.
#[cfg(target_os = "macos")]
fn bsd_info(pid: u32) -> Result<libc::proc_bsdinfo, i32> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: the buffer is a proc_bsdinfo and `size` is its length.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if written == size {
        // SAFETY: the kernel filled the whole struct.
        Ok(unsafe { info.assume_init() })
    } else {
        Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
    }
}

#[cfg(target_os = "macos")]
fn is_zombie(pid: u32) -> bool {
    match bsd_info(pid) {
        Ok(info) => info.pbi_status == libc::SZOMB,
        // kill(2) still sees it but it has no process info left: it exited
        // and waits to be reaped.
        Err(errno) => errno == libc::ESRCH,
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn is_zombie(_pid: u32) -> bool {
    false
}

/// Linux: field 22 of `/proc/<pid>/stat`.
#[cfg(target_os = "linux")]
fn start_time(pid: u32) -> Option<u64> {
    // Fields after the name start at field 3, so field 22 is index 19.
    proc_stat(pid)?.get(19)?.parse().ok()
}

/// macOS: when the process started, in microseconds since the epoch.
#[cfg(target_os = "macos")]
fn start_time(pid: u32) -> Option<u64> {
    let info = bsd_info(pid).ok()?;
    Some(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
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

    #[cfg(unix)]
    #[test]
    fn children_get_signals_we_block_or_ignore() {
        // Runs `spawn` on a thread of its own, since the mask is per thread.
        fn sleeper(prepare: fn()) -> Child {
            std::thread::spawn(move || {
                prepare();
                let dir = tempfile::tempdir().unwrap();
                let mut cmd = Command::new("sleep");
                cmd.arg("30");
                spawn_detached(cmd, &dir.path().join("log")).unwrap()
            })
            .join()
            .unwrap()
        }
        fn block_usr1() {
            // SAFETY: plain sigprocmask(2) on this thread.
            unsafe {
                let mut set = std::mem::zeroed::<libc::sigset_t>();
                libc::sigemptyset(&mut set);
                libc::sigaddset(&mut set, libc::SIGUSR1);
                libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
            }
        }
        fn ignore_usr2() {
            // SAFETY: plain signal(2); nothing in the tests sends us SIGUSR2.
            unsafe { libc::signal(libc::SIGUSR2, libc::SIG_IGN) };
        }

        for (prepare, signal) in [
            (block_usr1 as fn(), libc::SIGUSR1),
            (ignore_usr2 as fn(), libc::SIGUSR2),
        ] {
            let mut child = sleeper(prepare);
            // Let the child get past exec.
            std::thread::sleep(Duration::from_millis(200));
            // SAFETY: plain kill(2) on our own child.
            unsafe { libc::kill(child.id() as libc::pid_t, signal) };
            let started = Instant::now();
            let status = child.wait().unwrap();
            assert!(!status.success(), "signal {signal} ended the child");
            assert!(started.elapsed() < Duration::from_secs(5));
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn knows_start_times_and_zombies() {
        let dir = tempfile::tempdir().unwrap();
        let ours = Tracked::of(std::process::id());
        assert!(ours.started.is_some());
        assert!(ours.is_alive());

        // Exited but not waited for yet: a zombie, which is not running.
        let mut child = spawn_detached(Command::new("true"), &dir.path().join("log")).unwrap();
        let tracked = Tracked::of(child.id());
        let deadline = Instant::now() + Duration::from_secs(5);
        while tracked.is_alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!tracked.is_alive());
        child.wait().unwrap();
    }
}
