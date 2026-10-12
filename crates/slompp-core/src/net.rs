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

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::Listener;

    /// Not implemented yet: port checks still work through [`super::port_open`].
    pub fn listeners() -> Vec<Listener> {
        Vec::new()
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:0277 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 25145 1 0000000000000000 100 0 0 10 0
   1: 0100007F:1F90 0100007F:D2A4 01 00000000:00000000 00:00000000 00000000  1000        0 0 1 0000000000000000 20 4 30 10 -1
";

    const TCP6: &str = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000001000000:0277 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 25146 1 0000000000000000 100 0 0 10 0
";

    #[test]
    fn parses_listening_sockets_only() {
        let v4 = imp::parse_proc_net_tcp(TCP, false);
        assert_eq!(v4, vec![("127.0.0.1:631".parse().unwrap(), 25145)]);
        let v6 = imp::parse_proc_net_tcp(TCP6, true);
        assert_eq!(v6, vec![("[::1]:631".parse().unwrap(), 25146)]);
    }

    #[test]
    fn unwraps_nix_program_names() {
        assert_eq!(imp::unwrap_nix_name(".kdeconnectd-wrapped"), "kdeconnectd");
        assert_eq!(imp::unwrap_nix_name(".Discord-wrappe"), "Discord");
        assert_eq!(imp::unwrap_nix_name("httpd"), "httpd");
        assert_eq!(imp::unwrap_nix_name(".hidden"), ".hidden");
    }

    #[test]
    fn sees_our_own_listener() {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        assert!(port_open(port));
        let owner = owner_of(port).expect("listener visible");
        assert_eq!(owner.pid, Some(std::process::id()));
        drop(socket);
        assert!(!port_open(port));
    }
}
