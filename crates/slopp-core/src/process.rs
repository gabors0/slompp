//! Starting server processes so they outlive the panel, remembering them in
//! pid files, and telling whether they are still the processes we started.

use std::ffi::OsStr;
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
    /// microseconds since the epoch; Windows: the creation `FILETIME`), if
    /// known.
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
    let program = cmd.get_program().to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
        use windows_sys::Win32::System::Threading::{
            CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW,
        };
        // A console of its own that nobody sees, and no ^C from ours.
        let flags = CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW;
        // Leave the panel's job object, if it is in one: a terminal or an IDE
        // may kill the whole job when it closes. Not every job allows that.
        cmd.creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB);
        match cmd.spawn() {
            Err(err) if err.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
                cmd.creation_flags(flags);
            }
            spawned => return spawned.with_context(|| format!("starting {program}")),
        }
    }
    cmd.spawn().with_context(|| format!("starting {program}"))
}

/// A command for a helper that runs briefly (`httpd -t`, `mariadbd
/// --version`). On Windows it gets no console window, which would otherwise
/// flash up from the panel.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    cmd
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
    let mut killed = false;
    let mut asked = Vec::new();
    for p in procs.iter().filter(|p| p.is_alive()) {
        if signal_term(p.pid) {
            asked.push(*p);
        } else {
            // No way to ask this one.
            signal_kill(p.pid);
            killed = true;
        }
    }
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if asked.iter().all(|p| !p.is_alive()) {
            return killed;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    for p in asked.iter().filter(|p| p.is_alive()) {
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

/// Windows: the process's creation time.
#[cfg(windows)]
fn start_time(pid: u32) -> Option<u64> {
    win::Process::open(pid, win::QUERY)?.started()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn start_time(_pid: u32) -> Option<u64> {
    None
}

/// Asks a process to exit. Returns false if there is no way to ask it.
#[cfg(unix)]
fn signal_term(pid: u32) -> bool {
    // SAFETY: plain kill(2).
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    true
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
    match win::Process::open(pid, win::QUERY) {
        Some(process) => process.running(),
        // Someone else's process: it exists, but it isn't one we started.
        None => win::last_error() == windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED,
    }
}

/// Windows has no SIGTERM. The servers stop cleanly when a named event of
/// theirs is set: Apache's `ap<pid>_shutdown` (what `httpd -k shutdown`
/// sets) and MariaDB's `MySQLShutdown<pid>`.
#[cfg(windows)]
fn signal_term(pid: u32) -> bool {
    win::set_event(&format!("ap{pid}_shutdown")) || win::set_event(&format!("MySQLShutdown{pid}"))
}

/// Ends the process and everything it started: Apache's worker process
/// would otherwise keep the ports.
#[cfg(windows)]
fn signal_kill(pid: u32) {
    for pid in win::tree(pid) {
        if let Some(process) = win::Process::open(pid, win::TERMINATE) {
            process.terminate();
        }
    }
}

/// The Win32 calls behind the functions above, and [`crate::net`]'s process
/// names.
#[cfg(windows)]
pub(crate) mod win {
    use std::os::windows::ffi::OsStringExt as _;
    use std::path::PathBuf;

    use windows_sys::Win32::Foundation::{
        CloseHandle, FILETIME, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        EVENT_MODIFY_STATE, GetProcessTimes, OpenEventW, OpenProcess, PROCESS_ACCESS_RIGHTS,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        PROCESS_TERMINATE, QueryFullProcessImageNameW, SetEvent, TerminateProcess,
        WaitForSingleObject,
    };

    pub const QUERY: PROCESS_ACCESS_RIGHTS =
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
    pub const TERMINATE: PROCESS_ACCESS_RIGHTS = PROCESS_TERMINATE;

    pub fn last_error() -> u32 {
        // SAFETY: no preconditions.
        unsafe { GetLastError() }
    }

    /// A handle, closed on drop.
    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: we own the handle.
            unsafe { CloseHandle(self.0) };
        }
    }

    pub struct Process(Handle);

    impl Process {
        pub fn open(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Option<Process> {
            // SAFETY: plain OpenProcess; a null handle means failure.
            let handle = unsafe { OpenProcess(access, 0, pid) };
            (!handle.is_null()).then(|| Process(Handle(handle)))
        }

        /// Not exited yet (an exited process stays openable while someone
        /// holds a handle to it, like a zombie).
        pub fn running(&self) -> bool {
            // SAFETY: a process handle opened with SYNCHRONIZE.
            unsafe { WaitForSingleObject(self.0.0, 0) == WAIT_TIMEOUT }
        }

        pub fn started(&self) -> Option<u64> {
            let mut times = [FILETIME::default(); 4];
            let [created, exited, kernel, user] = &mut times;
            // SAFETY: four valid FILETIMEs to fill.
            let ok = unsafe { GetProcessTimes(self.0.0, created, exited, kernel, user) };
            (ok != 0)
                .then(|| u64::from(created.dwHighDateTime) << 32 | u64::from(created.dwLowDateTime))
        }

        /// The program's full path.
        pub fn image(&self) -> Option<PathBuf> {
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            // SAFETY: `len` is the buffer's length in characters.
            let ok = unsafe {
                QueryFullProcessImageNameW(self.0.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
            };
            (ok != 0).then(|| std::ffi::OsString::from_wide(&buf[..len as usize]).into())
        }

        pub fn terminate(&self) {
            // SAFETY: a process handle opened with PROCESS_TERMINATE.
            unsafe { TerminateProcess(self.0.0, 1) };
        }
    }

    pub fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    /// Sets the named event in this session, if it exists.
    pub fn set_event(name: &str) -> bool {
        let name = wide(name);
        // SAFETY: a NUL-terminated name.
        let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
        if event.is_null() {
            return false;
        }
        let event = Handle(event);
        // SAFETY: an event handle opened for EVENT_MODIFY_STATE.
        unsafe { SetEvent(event.0) != 0 }
    }

    /// `pid` and all its descendants, parents first.
    pub fn tree(pid: u32) -> Vec<u32> {
        let all = processes();
        let mut tree = vec![pid];
        let mut i = 0;
        while i < tree.len() {
            let parent = tree[i];
            let parent_started = Process::open(parent, QUERY).and_then(|p| p.started());
            for &(child, of) in &all {
                if of != parent || tree.contains(&child) {
                    continue;
                }
                // Parent ids aren't cleared when the parent exits, and pids
                // are reused: a real child started after its parent.
                let child_started = Process::open(child, QUERY).and_then(|p| p.started());
                if let (Some(parent), Some(child)) = (parent_started, child_started)
                    && child < parent
                {
                    continue;
                }
                tree.push(child);
            }
            i += 1;
        }
        tree
    }

    /// `(pid, parent pid)` of every process.
    fn processes() -> Vec<(u32, u32)> {
        // SAFETY: plain snapshot call; INVALID_HANDLE_VALUE means failure.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let snapshot = Handle(snapshot);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut all = Vec::new();
        // SAFETY: `entry` is a PROCESSENTRY32W with its size set.
        let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
        while more {
            all.push((entry.th32ProcessID, entry.th32ParentProcessID));
            // SAFETY: as above.
            more = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
        }
        all
    }
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

    /// Something to run in the background on Windows, which has no `sleep`:
    /// this test binary running only this test.
    #[cfg(windows)]
    #[test]
    #[ignore = "a child process for the tests below"]
    fn sleeper() {
        std::thread::sleep(Duration::from_secs(30));
    }

    #[cfg(windows)]
    fn sleeper_command() -> String {
        let exe = std::env::current_exe().unwrap();
        format!(
            "\"{}\" --ignored --exact process::tests::sleeper",
            exe.display()
        )
    }

    #[cfg(windows)]
    #[test]
    fn kills_a_detached_process_tree() {
        let dir = tempfile::tempdir().unwrap();
        // cmd, and the sleeper as its child.
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", &sleeper_command()]);
        let mut child = spawn_detached(cmd, &dir.path().join("log")).unwrap();
        let tracked = Tracked::of(child.id());
        assert!(tracked.is_alive());
        let started = tracked.started.expect("a start time");
        let stale = Tracked {
            pid: tracked.pid,
            started: Some(started + 1),
        };
        assert!(!stale.is_alive());

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut tree = win::tree(tracked.pid);
        while tree.len() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            tree = win::tree(tracked.pid);
        }
        assert_eq!(tree.len(), 2, "cmd and its child");
        let grandchild = Tracked::of(tree[1]);

        // Neither has a shutdown event to ask: both are killed right away.
        let asked = Instant::now();
        assert!(terminate(&[tracked], Duration::from_secs(20)));
        assert!(asked.elapsed() < Duration::from_secs(5));
        child.wait().unwrap();
        assert!(!tracked.is_alive());
        let deadline = Instant::now() + Duration::from_secs(5);
        while grandchild.is_alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!grandchild.is_alive());
    }

    #[cfg(windows)]
    #[test]
    fn asks_servers_to_stop_through_their_events() {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

        let pid = std::process::id();
        assert!(!signal_term(pid));
        for name in [format!("ap{pid}_shutdown"), format!("MySQLShutdown{pid}")] {
            let name = win::wide(&name);
            // SAFETY: a NUL-terminated name; the handle is closed below.
            unsafe {
                let event = CreateEventW(std::ptr::null(), 1, 0, name.as_ptr());
                assert!(!event.is_null());
                assert!(signal_term(pid));
                assert_eq!(WaitForSingleObject(event, 0), WAIT_OBJECT_0);
                CloseHandle(event);
            }
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
