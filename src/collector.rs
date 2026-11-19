use std::time::Instant;
use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, ProcessRefreshKind, RefreshKind, System, UpdateKind,
};

#[derive(Debug, Clone, Default)]
pub struct ProcRow {
    pub pid: u32,
    pub name: String,
    pub cpu_pct: f32,
    pub ram_pct: f32,
    pub mem_bytes: u64,
    /// Disk reads and writes since the previous snapshot, per second.
    pub read_per_sec: u64,
    pub write_per_sec: u64,
    pub run_time_secs: u64,
    /// "running", "sleeping", "zombie", …
    pub state: String,
    /// Listening TCP ports for this pid (filled in by the caller via the
    /// `ports` module; empty when not collected).
    pub ports: Vec<u16>,
    /// Working directory with the home folder shown as `~`; empty when the
    /// system does not allow reading it.
    pub cwd: String,
}

/// `/Users/me/app` → `~/app` for a home of `/Users/me`.
fn tilde_path(path: &str, home: &str) -> String {
    if home.is_empty() {
        return path.to_string();
    }
    match path.strip_prefix(home) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// System-wide summary shown in the header line.
#[derive(Debug, Clone, Default)]
pub struct SysStats {
    /// 1, 5 and 15 minute load averages.
    pub load1: f64,
    pub load5: f64,
    pub load15: f64,
    /// Global CPU usage across all cores, 0..=100.
    pub cpu_pct: f32,
    /// Used memory as a percentage of total, 0..=100.
    pub mem_pct: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub uptime_secs: u64,
    pub cores: usize,
    /// Number of processes seen in the last snapshot.
    pub proc_count: usize,
}

pub struct Collector {
    sys: System,
    total_mem: u64,
    last_snapshot: Instant,
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector {
    pub fn new() -> Self {
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_memory(MemoryRefreshKind::everything())
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_processes(ProcessRefreshKind::everything()),
        );
        sys.refresh_memory();
        let total_mem = sys.total_memory().max(1);
        Collector {
            sys,
            total_mem,
            last_snapshot: Instant::now(),
        }
    }

    /// System-wide summary. Global CPU usage is meaningful only after two
    /// refreshes spaced apart; the 1.5s event loop satisfies that.
    pub fn system_stats(&mut self) -> SysStats {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        let used = self.sys.used_memory();
        let load = System::load_average();
        SysStats {
            load1: load.one,
            load5: load.five,
            load15: load.fifteen,
            cpu_pct: self.sys.global_cpu_usage(),
            mem_pct: (used as f64 / self.total_mem as f64 * 100.0) as f32,
            mem_used: used,
            mem_total: self.total_mem,
            swap_used: self.sys.used_swap(),
            swap_total: self.sys.total_swap(),
            uptime_secs: System::uptime(),
            cores: self.sys.cpus().len(),
            proc_count: self.sys.processes().len(),
        }
    }

    pub fn snapshot(&mut self) -> Vec<ProcRow> {
        // Same kinds as `refresh_processes`, plus the working directory (read once per process).
        self.sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_memory()
                .with_cpu()
                .with_disk_usage()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cwd(UpdateKind::OnlyIfNotSet),
        );
        let home = std::env::var("HOME").unwrap_or_default();
        let elapsed_secs = self.last_snapshot.elapsed().as_secs_f64().max(0.001);
        self.last_snapshot = Instant::now();
        // Some system processes report no start time, which counts from 1970.
        let uptime_secs = System::uptime();

        let mut rows: Vec<ProcRow> = self
            .sys
            .processes()
            .iter()
            .map(|(pid, p)| ProcRow {
                pid: pid.as_u32(),
                name: p.name().to_string_lossy().to_string(),
                cpu_pct: p.cpu_usage(),
                ram_pct: (p.memory() as f64 / self.total_mem as f64 * 100.0) as f32,
                mem_bytes: p.memory(),
                read_per_sec: (p.disk_usage().read_bytes as f64 / elapsed_secs) as u64,
                write_per_sec: (p.disk_usage().written_bytes as f64 / elapsed_secs) as u64,
                run_time_secs: p.run_time().min(uptime_secs),
                state: p.status().to_string().to_lowercase(),
                ports: Vec::new(),
                cwd: p
                    .cwd()
                    .map(|path| tilde_path(&path.to_string_lossy(), &home))
                    .unwrap_or_default(),
            })
            .collect();

        rows.sort_by(|a, b| {
            b.cpu_pct
                .partial_cmp(&a.cpu_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn snapshot_returns_rows_sorted_by_cpu_desc() {
        let mut c = Collector::new();
        // First snapshot primes CPU deltas; sysinfo needs two reads.
        let _ = c.snapshot();
        sleep(Duration::from_millis(300));
        let rows = c.snapshot();

        assert!(!rows.is_empty(), "should see running processes");
        // sorted descending by cpu
        for pair in rows.windows(2) {
            assert!(
                pair[0].cpu_pct >= pair[1].cpu_pct,
                "rows must be sorted by cpu desc: {} then {}",
                pair[0].cpu_pct,
                pair[1].cpu_pct
            );
        }
        // ram_pct is a sane percentage
        for r in &rows {
            assert!(r.ram_pct >= 0.0 && r.ram_pct <= 100.0, "ram% in range");
        }
    }

    #[test]
    fn tilde_path_replaces_only_a_whole_home_folder() {
        assert_eq!(tilde_path("/Users/me", "/Users/me"), "~");
        assert_eq!(tilde_path("/Users/me/app", "/Users/me"), "~/app");
        assert_eq!(tilde_path("/Users/mel/app", "/Users/me"), "/Users/mel/app");
        assert_eq!(tilde_path("/tmp", ""), "/tmp");
    }

    #[test]
    fn snapshot_reads_the_working_directory_of_this_process() {
        let mut c = Collector::new();
        let rows = c.snapshot();
        let own = rows.iter().find(|r| r.pid == std::process::id()).unwrap();
        assert!(!own.cwd.is_empty(), "own cwd should be readable");
    }

    #[test]
    fn system_stats_returns_sane_values() {
        let mut c = Collector::new();
        // Prime CPU deltas, then read again.
        let _ = c.system_stats();
        sleep(Duration::from_millis(300));
        let s = c.system_stats();

        assert!(s.load1 >= 0.0, "load average is non-negative");
        assert!(
            s.cpu_pct >= 0.0 && s.cpu_pct <= 100.5,
            "global cpu% in range"
        );
        assert!(s.mem_pct >= 0.0 && s.mem_pct <= 100.0, "mem% in range");
        assert!(s.proc_count > 0, "should see at least one process");
    }
}
