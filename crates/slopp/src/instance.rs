//! One SLOPP per user. A second launch (from the app menu, say, while the
//! first sits in the tray) asks the running one to show its window and exits.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use crate::tray::TrayCommand;

pub enum Instance {
    /// We're the first; the listener answers later launches.
    Primary(UnixListener),
    /// Another SLOPP is running and was asked to show itself.
    Secondary,
}

fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    // Without XDG_RUNTIME_DIR the temp dir is shared between users.
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    dir.join(format!("slopp-{uid}.sock"))
}

/// Becomes the running instance, or hands over to the one already running.
pub fn claim() -> anyhow::Result<Instance> {
    let path = socket_path();
    if let Ok(mut stream) = UnixStream::connect(&path) {
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        stream.write_all(b"show\n")?;
        return Ok(Instance::Secondary);
    }
    // Nobody answered: whatever is there is left over from a crash.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    Ok(Instance::Primary(listener))
}

/// Turns "show" requests from later launches into [`TrayCommand::ShowPanel`].
pub fn serve(listener: UnixListener, commands: async_channel::Sender<TrayCommand>) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            if BufReader::new(stream).read_line(&mut line).is_ok() && line.trim() == "show" {
                let _ = commands.send_blocking(TrayCommand::ShowPanel);
            }
        }
    });
}

/// Removes the socket on a clean exit.
pub fn release() {
    let _ = std::fs::remove_file(socket_path());
}
