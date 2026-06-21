//! Listening-TCP-port discovery via `lsof`.
//!
//! Used both to annotate the process list with the ports each process listens
//! on, and to drive the `--port` / `--kill-port` command-line helpers.

use std::collections::HashMap;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listener {
    pub pid: u32,
    pub command: String,
    pub port: u16,
}

/// Extract the port from an lsof address field such as `*:3000`,
/// `127.0.0.1:8080`, or `[::1]:5432`.
fn port_from_addr(addr: &str) -> Option<u16> {
    // LISTEN sockets have no "->peer" part, but guard against it anyway.
    let local = addr.split("->").next().unwrap_or(addr);
    local.rsplit(':').next()?.parse().ok()
}

/// Parse `lsof -F pcn` output into one [`Listener`] per (pid, port).
///
/// Records are field-per-line: `p<pid>`, `c<command>`, `f<fd>`, `n<addr>`.
/// A `p` line starts a new process; each `n` line is one listening address.
pub fn parse_lsof(out: &str) -> Vec<Listener> {
    let mut listeners = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut pid = 0u32;
    let mut command = String::new();

    for line in out.lines() {
        let Some(tag) = line.chars().next() else {
            continue;
        };
        let val = &line[1..];
        match tag {
            'p' => pid = val.parse().unwrap_or(0),
            'c' => command = val.to_string(),
            'n' => {
                if let Some(port) = port_from_addr(val) {
                    if seen.insert((pid, port)) {
                        listeners.push(Listener { pid, command: command.clone(), port });
                    }
                }
            }
            _ => {}
        }
    }
    listeners
}

fn run_lsof(args: &[&str]) -> String {
    Command::new("lsof")
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Map of pid → sorted listening ports for every listening TCP socket.
pub fn ports_by_pid() -> HashMap<u32, Vec<u16>> {
    let out = run_lsof(&["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpcn"]);
    let mut map: HashMap<u32, Vec<u16>> = HashMap::new();
    for l in parse_lsof(&out) {
        let entry = map.entry(l.pid).or_default();
        if !entry.contains(&l.port) {
            entry.push(l.port);
        }
    }
    for ports in map.values_mut() {
        ports.sort_unstable();
    }
    map
}

/// Processes listening on a specific TCP port.
pub fn on_port(port: u16) -> Vec<Listener> {
    let arg = format!("-iTCP:{port}");
    let out = run_lsof(&["-nP", &arg, "-sTCP:LISTEN", "-Fpcn"]);
    parse_lsof(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pid_command_and_port() {
        let out = "p1234\ncnode\nf10\nn*:3000\n";
        let l = parse_lsof(out);
        assert_eq!(l, vec![Listener { pid: 1234, command: "node".into(), port: 3000 }]);
    }

    #[test]
    fn handles_multiple_ports_and_processes() {
        let out = "p1\ncnode\nf3\nn*:3000\nf4\nn127.0.0.1:8080\np2\ncpostgres\nf5\nn[::1]:5432\n";
        let l = parse_lsof(out);
        assert_eq!(l.len(), 3);
        assert!(l.contains(&Listener { pid: 1, command: "node".into(), port: 3000 }));
        assert!(l.contains(&Listener { pid: 1, command: "node".into(), port: 8080 }));
        assert!(l.contains(&Listener { pid: 2, command: "postgres".into(), port: 5432 }));
    }

    #[test]
    fn dedupes_repeated_pid_port_pairs() {
        // lsof prints the same listening socket once per fd (IPv4/IPv6).
        let out = "p596\ncrapportd\nf10\nn*:54801\nf11\nn*:54801\n";
        let l = parse_lsof(out);
        assert_eq!(l, vec![Listener { pid: 596, command: "rapportd".into(), port: 54801 }]);
    }

    #[test]
    fn ignores_unparseable_addresses() {
        let out = "p1\ncweird\nf3\nn*:notaport\n";
        assert!(parse_lsof(out).is_empty());
    }
}
