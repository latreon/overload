use sysinfo::{MemoryRefreshKind, ProcessRefreshKind, RefreshKind, System};

#[derive(Debug, Clone)]
pub struct ProcRow {
    pub pid: u32,
    pub name: String,
    pub cpu_pct: f32,
    pub ram_pct: f32,
}

pub struct Collector {
    sys: System,
    total_mem: u64,
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
                .with_processes(ProcessRefreshKind::everything()),
        );
        sys.refresh_memory();
        let total_mem = sys.total_memory().max(1);
        Collector { sys, total_mem }
    }

    pub fn snapshot(&mut self) -> Vec<ProcRow> {
        self.sys
            .refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        let mut rows: Vec<ProcRow> = self
            .sys
            .processes()
            .iter()
            .map(|(pid, p)| ProcRow {
                pid: pid.as_u32(),
                name: p.name().to_string_lossy().to_string(),
                cpu_pct: p.cpu_usage(),
                ram_pct: (p.memory() as f64 / self.total_mem as f64 * 100.0) as f32,
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
}
