//! Port checks: is something listening, and which program is it (XAMPP's
//! "Netstat" window and its "port 80 in use by ..." warnings).

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// Whether something accepts TCP connections on localhost:`port`.
pub fn port_open(port: u16) -> bool {
    let timeout = Duration::from_millis(250);
    [
        SocketAddr::from(([127, 0, 0, 1], port)),
        SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, port)),
    ]
    .iter()
    .any(|addr| TcpStream::connect_timeout(addr, timeout).is_ok())
}

/// A listening TCP socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listener {
    pub addr: SocketAddr,
    /// Owner, when the OS lets us see it (other users' processes are hidden).
    pub pid: Option<u32>,
    pub process: Option<String>,
}

impl Listener {
    pub fn port(&self) -> u16 {
        self.addr.port()
    }
}

/// Every listening TCP socket, sorted by port.
pub fn listeners() -> Vec<Listener> {
    let mut all = imp::listeners();
    all.sort_by_key(|l| (l.port(), l.addr.is_ipv6(), l.addr.ip()));
    all.dedup();
    all
}

/// Who is listening on `port`, if anyone.
pub fn owner_of(port: u16) -> Option<Listener> {
    let all = listeners();
    // Prefer an entry whose owner we can see.
    all.iter()
        .find(|l| l.port() == port && l.pid.is_some())
        .or_else(|| all.iter().find(|l| l.port() == port))
        .cloned()
}

#[cfg(target_os = "linux")]
mod imp {
    use super::Listener;
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    const TCP_LISTEN: &str = "0A";

    pub fn listeners() -> Vec<Listener> {
        let mut sockets = Vec::new();
        for (file, v6) in [("/proc/net/tcp", false), ("/proc/net/tcp6", true)] {
            if let Ok(text) = std::fs::read_to_string(file) {
                sockets.extend(parse_proc_net_tcp(&text, v6));
            }
        }
        let owners = socket_owners();
        sockets
            .into_iter()
            .map(|(addr, inode)| {
                let pid = owners.get(&inode).copied();
                Listener {
                    addr,
                    pid,
                    process: pid.and_then(process_name),
                }
            })
            .collect()
    }

    /// `(address, socket inode)` for each listening socket in a
    /// `/proc/net/tcp{,6}` table.
    pub(super) fn parse_proc_net_tcp(text: &str, v6: bool) -> Vec<(SocketAddr, u64)> {
        text.lines()
            .skip(1)
            .filter_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                if fields.get(3) != Some(&TCP_LISTEN) {
                    return None;
                }
                let (ip, port) = fields.get(1)?.split_once(':')?;
                let port = u16::from_str_radix(port, 16).ok()?;
                let ip = if v6 { parse_v6(ip)? } else { parse_v4(ip)? };
                let inode = fields.get(9)?.parse().ok()?;
                Some((SocketAddr::new(ip, port), inode))
            })
            .collect()
    }

    /// The kernel prints the address as a native-endian u32.
    fn parse_v4(hex: &str) -> Option<IpAddr> {
        let n = u32::from_str_radix(hex, 16).ok()?;
        Some(IpAddr::V4(Ipv4Addr::from(n.to_le_bytes())))
    }

    /// Four native-endian u32 words.
    fn parse_v6(hex: &str) -> Option<IpAddr> {
        if hex.len() != 32 {
            return None;
        }
        let mut bytes = [0u8; 16];
        for word in 0..4 {
            let n = u32::from_str_radix(&hex[word * 8..word * 8 + 8], 16).ok()?;
            bytes[word * 4..word * 4 + 4].copy_from_slice(&n.to_le_bytes());
        }
        Some(IpAddr::V6(Ipv6Addr::from(bytes)))
    }

    /// Socket inode → pid, for every process whose fds we may read.
    fn socket_owners() -> HashMap<u64, u32> {
        let mut owners = HashMap::new();
        let Ok(procs) = std::fs::read_dir("/proc") else {
            return owners;
        };
        for entry in procs.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else {
                continue;
            };
            for fd in fds.flatten() {
                if let Ok(target) = std::fs::read_link(fd.path())
                    && let Some(inode) = target
                        .to_str()
                        .and_then(|t| t.strip_prefix("socket:["))
                        .and_then(|t| t.strip_suffix(']'))
                        .and_then(|t| t.parse().ok())
                {
                    owners.entry(inode).or_insert(pid);
                }
            }
        }
        owners
    }

    fn process_name(pid: u32) -> Option<String> {
        // `comm` is cut at 15 characters, so prefer the executable's name.
        let name = std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .and_then(|exe| Some(exe.file_name()?.to_string_lossy().into_owned()))
            .or_else(|| {
                let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
                Some(comm.trim().to_string())
            })?;
        Some(unwrap_nix_name(&name).to_string())
    }

    /// Nix wraps programs as `.foo-wrapped`; show `foo`. Copes with `comm`'s
    /// truncation (`.foo-wrapp`).
    pub(super) fn unwrap_nix_name(name: &str) -> &str {
        let Some(inner) = name.strip_prefix('.') else {
            return name;
        };
        if let Some(cut) = inner.rfind("-wr")
            && "-wrapped".starts_with(&inner[cut..])
            && cut > 0
        {
            return &inner[..cut];
        }
        name
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Listener;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    use std::process::Command;

    /// macOS has no `/proc`, and `netstat` hides TCP sockets from callers
    /// that aren't Apple's own programs, so ask `lsof`. Like `/proc/<pid>/fd`
    /// it only shows this user's processes: a port held by someone else's
    /// (the system's Apache, say) is still caught by [`super::port_open`],
    /// just without a name.
    pub fn listeners() -> Vec<Listener> {
        let Ok(out) = Command::new("/usr/sbin/lsof")
            .args(["+c", "0", "-nP", "-iTCP", "-sTCP:LISTEN", "-F", "pctn"])
            .output()
        else {
            return Vec::new();
        };
        let mut sockets = parse_lsof(&String::from_utf8_lossy(&out.stdout));
        // Forked workers (Apache's) hold the same socket as their parent:
        // show it once, under the oldest process.
        sockets.sort_by_key(|(addr, pid, _)| (*addr, *pid));
        sockets.dedup_by_key(|(addr, ..)| *addr);
        sockets
            .into_iter()
            .map(|(addr, pid, command)| Listener {
                addr,
                pid: Some(pid),
                process: process_name(pid).or(command),
            })
            .collect()
    }

    /// `(address, pid, command)` for each socket in `lsof -F pctn` output:
    /// one field per line, a `p` line opening each process and an `n` line
    /// closing each of its sockets.
    pub(super) fn parse_lsof(text: &str) -> Vec<(SocketAddr, u32, Option<String>)> {
        let mut sockets = Vec::new();
        let (mut pid, mut command, mut v6) = (None, None, false);
        for line in text.lines() {
            let Some(field) = line.chars().next() else {
                continue;
            };
            let value = &line[field.len_utf8()..];
            match field {
                'p' => {
                    pid = value.parse::<u32>().ok();
                    command = None;
                }
                'c' => command = Some(value.to_string()),
                't' => v6 = value == "IPv6",
                'n' => {
                    if let (Some(pid), Some(addr)) = (pid, parse_addr(value, v6)) {
                        sockets.push((addr, pid, command.clone()));
                    }
                }
                _ => {}
            }
        }
        sockets
    }

    /// `127.0.0.1:3306`, `[::1]:3306` or `*:80`.
    fn parse_addr(text: &str, v6: bool) -> Option<SocketAddr> {
        let (host, port) = text.rsplit_once(':')?;
        let port = port.parse().ok()?;
        let ip = if host == "*" {
            if v6 {
                IpAddr::V6(Ipv6Addr::UNSPECIFIED)
            } else {
                IpAddr::V4(Ipv4Addr::UNSPECIFIED)
            }
        } else {
            let host = host.trim_start_matches('[').trim_end_matches(']');
            // Drop the scope of a link-local address.
            host.split('%').next()?.parse().ok()?
        };
        Some(SocketAddr::new(ip, port))
    }

    /// The executable's file name, which unlike lsof's command isn't the
    /// name a program gave itself.
    fn process_name(pid: u32) -> Option<String> {
        let mut buf = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: the buffer is `buf.len()` bytes long.
        let len = unsafe {
            libc::proc_pidpath(
                pid as libc::c_int,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
            )
        };
        if len <= 0 {
            return None;
        }
        let path = String::from_utf8_lossy(&buf[..len as usize]).into_owned();
        let name = std::path::Path::new(&path).file_name()?;
        Some(name.to_string_lossy().into_owned())
    }
}

#[cfg(windows)]
mod imp {
    use super::Listener;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID,
        TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    use crate::process::win::{Process, QUERY};

    /// The kernel's listening sockets with their owners. Processes of other
    /// users and the system can't be opened, so they go without a name,
    /// except the kernel itself: http.sys (IIS, WinRM, ...) listens as PID 4.
    pub fn listeners() -> Vec<Listener> {
        let mut sockets = Vec::new();
        for row in table::<MIB_TCPROW_OWNER_PID>(AF_INET) {
            // Both in network byte order; the port in the low 16 bits.
            let ip = Ipv4Addr::from(u32::from_be(row.dwLocalAddr));
            let port = u16::from_be(row.dwLocalPort as u16);
            sockets.push((SocketAddr::new(IpAddr::V4(ip), port), row.dwOwningPid));
        }
        for row in table::<MIB_TCP6ROW_OWNER_PID>(AF_INET6) {
            let ip = Ipv6Addr::from(row.ucLocalAddr);
            let port = u16::from_be(row.dwLocalPort as u16);
            sockets.push((SocketAddr::new(IpAddr::V6(ip), port), row.dwOwningPid));
        }
        sockets
            .into_iter()
            .map(|(addr, pid)| Listener {
                addr,
                pid: Some(pid),
                process: process_name(pid),
            })
            .collect()
    }

    /// The rows of `GetExtendedTcpTable`'s listener table for `family`.
    /// `Row` is the table's row type; the table is a `u32` count followed by
    /// the rows.
    fn table<Row: Copy>(family: u16) -> Vec<Row> {
        let mut size = 0u32;
        let mut buf: Vec<u64> = Vec::new();
        // The table can grow between asking for its size and reading it.
        for _ in 0..4 {
            // SAFETY: `buf` holds `size` bytes, 8-aligned for the rows.
            let err = unsafe {
                GetExtendedTcpTable(
                    buf.as_mut_ptr().cast(),
                    &mut size,
                    0,
                    u32::from(family),
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            match err {
                NO_ERROR => {
                    let base = buf.as_ptr().cast::<u8>();
                    // SAFETY: the API wrote a count then that many rows, the
                    // rows starting at the row type's alignment.
                    unsafe {
                        let count = base.cast::<u32>().read() as usize;
                        let offset = std::mem::align_of::<Row>().max(4);
                        let rows = base.add(offset).cast::<Row>();
                        return (0..count).map(|i| rows.add(i).read()).collect();
                    }
                }
                ERROR_INSUFFICIENT_BUFFER => buf = vec![0; (size as usize).div_ceil(8)],
                _ => break,
            }
        }
        Vec::new()
    }

    fn process_name(pid: u32) -> Option<String> {
        if pid == 4 {
            return Some("System".to_string());
        }
        let image = Process::open(pid, QUERY)?.image()?;
        Some(image.file_name()?.to_string_lossy().into_owned())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod imp {
    use super::Listener;

    /// Not implemented yet: port checks still work through [`super::port_open`].
    pub fn listeners() -> Vec<Listener> {
        Vec::new()
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:0277 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 25145 1 0000000000000000 100 0 0 10 0
   1: 0100007F:1F90 0100007F:D2A4 01 00000000:00000000 00:00000000 00000000  1000        0 0 1 0000000000000000 20 4 30 10 -1
";

    #[cfg(target_os = "linux")]
    const TCP6: &str = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:0277 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 25146 1 0000000000000000 100 0 0 10 0
";

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_listening_sockets_only() {
        let v4 = imp::parse_proc_net_tcp(TCP, false);
        assert_eq!(v4, vec![("127.0.0.1:631".parse().unwrap(), 25145)]);
        let v6 = imp::parse_proc_net_tcp(TCP6, true);
        assert_eq!(v6, vec![("[::1]:631".parse().unwrap(), 25146)]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unwraps_nix_program_names() {
        assert_eq!(imp::unwrap_nix_name(".kdeconnectd-wrapped"), "kdeconnectd");
        assert_eq!(imp::unwrap_nix_name(".Discord-wrappe"), "Discord");
        assert_eq!(imp::unwrap_nix_name("httpd"), "httpd");
        assert_eq!(imp::unwrap_nix_name(".hidden"), ".hidden");
    }

    #[cfg(target_os = "macos")]
    const LSOF: &str = "p654
cControlCenter
f9
tIPv4
n*:7000
f10
tIPv6
n*:7000
p31376
cnode
f24
tIPv4
n127.0.0.1:7265
p4242
cmariadbd
f30
tIPv6
n[::1]:3306
f31
tIPv6
n[fe80::1%lo0]:3307
";

    #[cfg(target_os = "macos")]
    #[test]
    fn parses_lsof_fields() {
        let listener = |addr: &str, pid, command: &str| {
            (addr.parse().unwrap(), pid, Some(command.to_string()))
        };
        assert_eq!(
            imp::parse_lsof(LSOF),
            vec![
                listener("0.0.0.0:7000", 654, "ControlCenter"),
                listener("[::]:7000", 654, "ControlCenter"),
                listener("127.0.0.1:7265", 31376, "node"),
                listener("[::1]:3306", 4242, "mariadbd"),
                listener("[fe80::1]:3307", 4242, "mariadbd"),
            ]
        );
    }

    #[test]
    fn sees_our_own_listener() {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        assert!(port_open(port));
        let owner = owner_of(port).expect("listener visible");
        assert_eq!(owner.pid, Some(std::process::id()));
        assert!(owner.process.is_some());
        drop(socket);
        assert!(!port_open(port));
    }
}
