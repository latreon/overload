//! Listening-TCP-port discovery via `lsof`.
//!
//! Used both to annotate the process list with the ports each process listens
//! on, and to drive the `--port` / `--kill-port` command-line helpers.

use std::collections::HashMap;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

/// Absolute path to `lsof` on macOS. Hardcoded (rather than resolved via
/// `PATH`) so a poisoned `PATH` can't trick us into running a rogue binary
/// whose output we then feed straight into process-killing.
const LSOF_PATH: &str = "/usr/sbin/lsof";

/// Hard cap on how long we wait for `lsof`. `lsof` is a notorious hang
/// offender (stale NFS/FUSE mounts, slow name resolution); without a timeout a
/// single stall would freeze the whole TUI on every refresh tick.
const LSOF_TIMEOUT: Duration = Duration::from_millis(1500);

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
                        listeners.push(Listener {
                            pid,
                            command: command.clone(),
                            port,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    listeners
}

/// Run `lsof` with a hard timeout. The command runs on a worker thread whose
/// `output()` call drains the pipe (so no large-output deadlock), and we wait
/// on the result with [`LSOF_TIMEOUT`]. On timeout we return empty and let the
/// orphaned `lsof` exit on its own — the UI never blocks waiting for it.
fn run_lsof(args: &[&str]) -> Option<String> {
    let owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let out = Command::new(LSOF_PATH)
            .args(&owned)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .ok();
        let _ = tx.send(out);
    });
    rx.recv_timeout(LSOF_TIMEOUT).ok().flatten()
}

/// Map of pid → sorted listening ports for every listening TCP socket.
pub fn ports_by_pid() -> HashMap<u32, Vec<u16>> {
    let out = run_lsof(&["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpcn"]).unwrap_or_default();
    let mut map: HashMap<u32, Vec<u16>> = HashMap::new();
    // `parse_lsof` already dedupes (pid, port) pairs, so a plain push suffices.
    for l in parse_lsof(&out) {
        map.entry(l.pid).or_default().push(l.port);
    }
    for ports in map.values_mut() {
        ports.sort_unstable();
    }
    map
}

/// Processes listening on a specific TCP port.
/// `None` when `lsof` failed or timed out, so "no answer" is not read as "nothing listens".
pub fn on_port(port: u16) -> Option<Vec<Listener>> {
    let arg = format!("-iTCP:{port}");
    run_lsof(&["-nP", &arg, "-sTCP:LISTEN", "-Fpcn"]).map(|out| parse_lsof(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pid_command_and_port() {
        let out = "p1234\ncnode\nf10\nn*:3000\n";
        let l = parse_lsof(out);
        assert_eq!(
            l,
            vec![Listener {
                pid: 1234,
                command: "node".into(),
                port: 3000
            }]
        );
    }

    #[test]
    fn handles_multiple_ports_and_processes() {
        let out = "p1\ncnode\nf3\nn*:3000\nf4\nn127.0.0.1:8080\np2\ncpostgres\nf5\nn[::1]:5432\n";
        let l = parse_lsof(out);
        assert_eq!(l.len(), 3);
        assert!(l.contains(&Listener {
            pid: 1,
            command: "node".into(),
            port: 3000
        }));
        assert!(l.contains(&Listener {
            pid: 1,
            command: "node".into(),
            port: 8080
        }));
        assert!(l.contains(&Listener {
            pid: 2,
            command: "postgres".into(),
            port: 5432
        }));
    }

    #[test]
    fn dedupes_repeated_pid_port_pairs() {
        // lsof prints the same listening socket once per fd (IPv4/IPv6).
        let out = "p596\ncrapportd\nf10\nn*:54801\nf11\nn*:54801\n";
        let l = parse_lsof(out);
        assert_eq!(
            l,
            vec![Listener {
                pid: 596,
                command: "rapportd".into(),
                port: 54801
            }]
        );
    }

    #[test]
    fn ignores_unparseable_addresses() {
        let out = "p1\ncweird\nf3\nn*:notaport\n";
        assert!(parse_lsof(out).is_empty());
    }
}
