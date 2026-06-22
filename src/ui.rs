use crate::classifier::{classify, Protection};
use crate::collector::ProcRow;
use crossterm::event::KeyCode;
use std::collections::{HashMap, HashSet, VecDeque};

/// How many CPU samples to retain per row and for the global history.
pub const HISTORY_LEN: usize = 60;

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    /// Kill every pid in the selected row (one pid, or a whole app group).
    Kill(Vec<u32>, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Cpu,
    Ram,
    Name,
}

impl Sort {
    pub fn label(self) -> &'static str {
        match self {
            Sort::Cpu => "cpu",
            Sort::Ram => "ram",
            Sort::Name => "name",
        }
    }
}

/// One visible row: either a single process or an aggregated app group.
#[derive(Debug, Clone)]
pub struct DisplayRow {
    /// Stable identity used to keep the cursor on the same row across refreshes.
    pub key: String,
    pub label: String,
    pub cpu_pct: f32,
    pub ram_pct: f32,
    pub pids: Vec<u32>,
    pub count: usize,
    pub protected: bool,
    /// Listening TCP ports (union across the group's members), sorted.
    pub ports: Vec<u16>,
}

/// Collapse a process name to its app group by trimming helper/parenthetical
/// suffixes: "Google Chrome Helper (Renderer)" → "Google Chrome".
fn app_group(name: &str) -> String {
    let mut s = name;
    if let Some(i) = s.find(" Helper") {
        s = &s[..i];
    }
    if let Some(i) = s.find(" (") {
        s = &s[..i];
    }
    s.trim().to_string()
}

pub struct App {
    raw: Vec<ProcRow>,
    pub display: Vec<DisplayRow>,
    pub selected: usize,
    pub status: Option<String>,
    /// Issued kills not yet observed gone: (pids, label).
    pending: Vec<(Vec<u32>, String)>,
    pub grouped: bool,
    pub sort: Sort,
    pub filter: String,
    /// True while the user is typing a filter query.
    pub filtering: bool,
    /// Per-row CPU sample history, keyed by display-row key (oldest→newest).
    pub history: HashMap<String, VecDeque<f32>>,
    /// Global CPU usage history for the header sparkline (oldest→newest).
    pub cpu_history: VecDeque<f32>,
    /// Rows the user has marked for batch kill, by display-row key.
    pub marked: HashSet<String>,
}

impl App {
    pub fn new() -> Self {
        App {
            raw: Vec::new(),
            display: Vec::new(),
            selected: 0,
            status: None,
            pending: Vec::new(),
            grouped: true,
            sort: Sort::Cpu,
            filter: String::new(),
            filtering: false,
            history: HashMap::new(),
            cpu_history: VecDeque::new(),
            marked: HashSet::new(),
        }
    }

    /// Record one global CPU sample for the header sparkline.
    pub fn record_cpu(&mut self, cpu_pct: f32) {
        self.cpu_history.push_back(cpu_pct);
        while self.cpu_history.len() > HISTORY_LEN {
            self.cpu_history.pop_front();
        }
    }

    /// Record kills; status shows "killing…" until every pid disappears.
    pub fn register_kill(&mut self, pids: Vec<u32>, label: String) {
        self.status = Some(if pids.len() > 1 {
            format!("killing {label} ({} procs)…", pids.len())
        } else {
            format!("killing {label} ({})…", pids[0])
        });
        self.pending.push((pids, label));
    }

    pub fn set_rows(&mut self, rows: Vec<ProcRow>) {
        self.raw = rows;

        // Resolve pending kills against the processes still present.
        let present: std::collections::HashSet<u32> =
            self.raw.iter().map(|r| r.pid).collect();
        let pending = std::mem::take(&mut self.pending);
        for (pids, label) in pending {
            let alive: Vec<u32> = pids.into_iter().filter(|p| present.contains(p)).collect();
            if alive.is_empty() {
                self.status = Some(format!("✓ killed {label}"));
            } else {
                self.pending.push((alive, label));
            }
        }

        self.rebuild();
        self.sample_history();
    }

    /// Append the current CPU value to each visible row's history, carrying
    /// over prior samples and dropping rows that no longer exist.
    fn sample_history(&mut self) {
        let mut next: HashMap<String, VecDeque<f32>> = HashMap::new();
        for d in &self.display {
            let mut dq = self.history.remove(&d.key).unwrap_or_default();
            dq.push_back(d.cpu_pct);
            while dq.len() > HISTORY_LEN {
                dq.pop_front();
            }
            next.insert(d.key.clone(), dq);
        }
        self.history = next;
    }

    /// Recompute the visible rows from raw data, applying filter, grouping and
    /// sort, while keeping the cursor on the same logical row when possible.
    pub fn rebuild(&mut self) {
        let prev_key = self.display.get(self.selected).map(|d| d.key.clone());
        let q = self.filter.to_lowercase();

        let filtered = self
            .raw
            .iter()
            .filter(|r| q.is_empty() || r.name.to_lowercase().contains(&q));

        let mut display: Vec<DisplayRow> = if self.grouped {
            let mut map: HashMap<String, DisplayRow> = HashMap::new();
            for r in filtered {
                let key = app_group(&r.name);
                let protected = classify(&r.name, r.pid) == Protection::Protected;
                let entry = map.entry(key.clone()).or_insert_with(|| DisplayRow {
                    key: key.clone(),
                    label: key.clone(),
                    cpu_pct: 0.0,
                    ram_pct: 0.0,
                    pids: Vec::new(),
                    count: 0,
                    protected: false,
                    ports: Vec::new(),
                });
                entry.cpu_pct += r.cpu_pct;
                entry.ram_pct += r.ram_pct;
                entry.pids.push(r.pid);
                entry.count += 1;
                entry.protected = entry.protected || protected;
                for &p in &r.ports {
                    if !entry.ports.contains(&p) {
                        entry.ports.push(p);
                    }
                }
            }
            let mut rows: Vec<DisplayRow> = map.into_values().collect();
            for row in &mut rows {
                row.ports.sort_unstable();
            }
            rows
        } else {
            filtered
                .map(|r| DisplayRow {
                    key: format!("pid:{}", r.pid),
                    label: r.name.clone(),
                    cpu_pct: r.cpu_pct,
                    ram_pct: r.ram_pct,
                    pids: vec![r.pid],
                    count: 1,
                    protected: classify(&r.name, r.pid) == Protection::Protected,
                    ports: r.ports.clone(),
                })
                .collect()
        };

        match self.sort {
            Sort::Cpu => display.sort_by(|a, b| {
                b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(std::cmp::Ordering::Equal)
            }),
            Sort::Ram => display.sort_by(|a, b| {
                b.ram_pct.partial_cmp(&a.ram_pct).unwrap_or(std::cmp::Ordering::Equal)
            }),
            Sort::Name => {
                display.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
            }
        }

        self.display = display;

        // Keep the cursor on the same row identity; otherwise clamp.
        self.selected = match prev_key {
            Some(k) => self.display.iter().position(|d| d.key == k).unwrap_or(0),
            None => 0,
        };
        let max = self.display.len().saturating_sub(1);
        if self.selected > max {
            self.selected = max;
        }
    }

    pub fn handle_key(&mut self, code: KeyCode) -> Action {
        if self.filtering {
            return self.handle_filter_key(code);
        }

        let len = self.display.len();
        match code {
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Esc => {
                // Esc clears an active selection first; only quits when idle.
                if self.marked.is_empty() {
                    Action::Quit
                } else {
                    self.marked.clear();
                    Action::None
                }
            }
            KeyCode::Char(' ') => {
                if let Some(d) = self.display.get(self.selected) {
                    let key = d.key.clone();
                    if !self.marked.remove(&key) {
                        self.marked.insert(key);
                    }
                }
                Action::None
            }
            KeyCode::Char('/') => {
                self.filtering = true;
                Action::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.selected + 1 < len {
                    self.selected += 1;
                }
                Action::None
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                Action::None
            }
            KeyCode::Char('g') | KeyCode::Home => {
                self.selected = 0;
                Action::None
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.selected = len.saturating_sub(1);
                Action::None
            }
            KeyCode::PageDown => {
                self.selected = (self.selected + 10).min(len.saturating_sub(1));
                Action::None
            }
            KeyCode::PageUp => {
                self.selected = self.selected.saturating_sub(10);
                Action::None
            }
            KeyCode::Char('c') => {
                self.sort = Sort::Cpu;
                self.rebuild();
                Action::None
            }
            KeyCode::Char('m') => {
                self.sort = Sort::Ram;
                self.rebuild();
                Action::None
            }
            KeyCode::Char('n') => {
                self.sort = Sort::Name;
                self.rebuild();
                Action::None
            }
            KeyCode::Char('t') => {
                self.grouped = !self.grouped;
                self.rebuild();
                Action::None
            }
            KeyCode::Char('k') => {
                if !self.marked.is_empty() {
                    return self.kill_marked();
                }
                let Some(d) = self.display.get(self.selected) else {
                    return Action::None;
                };
                if d.protected {
                    self.status = Some(format!("{} is protected — not killed", d.label));
                    Action::None
                } else {
                    Action::Kill(d.pids.clone(), d.label.clone())
                }
            }
            _ => Action::None,
        }
    }

    /// Kill every marked, non-protected row. Clears the marks.
    fn kill_marked(&mut self) -> Action {
        let mut pids = Vec::new();
        let mut rows = 0usize;
        let mut skipped_protected = 0usize;
        for d in &self.display {
            if self.marked.contains(&d.key) {
                if d.protected {
                    skipped_protected += 1;
                } else {
                    pids.extend(&d.pids);
                    rows += 1;
                }
            }
        }
        self.marked.clear();
        if pids.is_empty() {
            self.status = Some(format!(
                "nothing killable selected ({skipped_protected} protected)"
            ));
            Action::None
        } else {
            Action::Kill(pids, format!("{rows} selected"))
        }
    }

    fn handle_filter_key(&mut self, code: KeyCode) -> Action {
        match code {
            KeyCode::Esc => {
                self.filtering = false;
                self.filter.clear();
                self.rebuild();
            }
            KeyCode::Enter => {
                self.filtering = false;
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.rebuild();
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.rebuild();
            }
            _ => {}
        }
        Action::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, name: &str, cpu: f32) -> ProcRow {
        ProcRow { pid, name: name.to_string(), cpu_pct: cpu, ram_pct: 0.0, ports: vec![] }
    }

    fn rowm(pid: u32, name: &str, cpu: f32, ram: f32) -> ProcRow {
        ProcRow { pid, name: name.to_string(), cpu_pct: cpu, ram_pct: ram, ports: vec![] }
    }

    fn app_with(rows: Vec<ProcRow>) -> App {
        let mut a = App::new();
        a.set_rows(rows);
        a
    }

    #[test]
    fn down_and_up_move_selection_within_bounds() {
        let mut a = app_with(vec![row(1, "alpha", 9.0), row(2, "bravo", 8.0)]);
        assert_eq!(a.selected, 0);
        a.handle_key(KeyCode::Down);
        assert_eq!(a.selected, 1);
        a.handle_key(KeyCode::Down); // clamp at bottom
        assert_eq!(a.selected, 1);
        a.handle_key(KeyCode::Up);
        assert_eq!(a.selected, 0);
        a.handle_key(KeyCode::Up); // clamp at top
        assert_eq!(a.selected, 0);
    }

    #[test]
    fn fresh_app_starts_selected_on_top_row() {
        // First populated snapshot must leave the cursor on row 0 (the hottest),
        // not somewhere in the middle.
        let a = app_with(vec![row(1, "low", 5.0), row(2, "hot", 90.0), row(3, "mid", 40.0)]);
        assert_eq!(a.selected, 0);
        assert_eq!(a.display[0].label, "hot", "row 0 is the highest-CPU process");
    }

    #[test]
    fn j_moves_down_like_arrow() {
        let mut a = app_with(vec![row(1, "alpha", 9.0), row(2, "bravo", 8.0)]);
        a.handle_key(KeyCode::Char('j'));
        assert_eq!(a.selected, 1);
    }

    #[test]
    fn g_and_shift_g_jump_to_ends() {
        let mut a = app_with(vec![row(1, "a", 9.0), row(2, "b", 8.0), row(3, "c", 7.0)]);
        a.handle_key(KeyCode::Char('G'));
        assert_eq!(a.selected, 2);
        a.handle_key(KeyCode::Char('g'));
        assert_eq!(a.selected, 0);
    }

    #[test]
    fn q_returns_quit() {
        let mut a = app_with(vec![row(500, "x", 1.0)]);
        assert_eq!(a.handle_key(KeyCode::Char('q')), Action::Quit);
    }

    #[test]
    fn k_on_killable_returns_kill_with_pids() {
        let mut a = app_with(vec![row(8981, "python", 99.0)]);
        assert_eq!(
            a.handle_key(KeyCode::Char('k')),
            Action::Kill(vec![8981], "python".to_string())
        );
    }

    #[test]
    fn k_on_protected_returns_none_and_sets_status() {
        let mut a = app_with(vec![row(50, "WindowServer", 42.0)]);
        assert_eq!(a.handle_key(KeyCode::Char('k')), Action::None);
        assert!(a.status.is_some());
    }

    #[test]
    fn grouped_aggregates_app_helpers() {
        let a = app_with(vec![
            row(1, "Google Chrome Helper (Renderer)", 10.0),
            row(2, "Google Chrome Helper (GPU)", 5.0),
            row(3, "python", 50.0),
        ]);
        // grouped is on by default: the two Chrome helpers collapse to one row.
        let chrome = a.display.iter().find(|d| d.label == "Google Chrome").unwrap();
        assert_eq!(chrome.count, 2);
        assert_eq!(chrome.cpu_pct, 15.0);
        assert_eq!(chrome.pids.len(), 2);
        // python stays its own row.
        assert!(a.display.iter().any(|d| d.label == "python" && d.count == 1));
    }

    #[test]
    fn killing_a_group_targets_all_its_pids() {
        let mut a = app_with(vec![
            row(1001, "Google Chrome Helper (Renderer)", 10.0),
            row(1002, "Google Chrome Helper (GPU)", 5.0),
        ]);
        // Chrome is the top row by CPU.
        match a.handle_key(KeyCode::Char('k')) {
            Action::Kill(pids, label) => {
                assert_eq!(label, "Google Chrome");
                let mut p = pids.clone();
                p.sort();
                assert_eq!(p, vec![1001, 1002]);
            }
            other => panic!("expected Kill, got {other:?}"),
        }
    }

    #[test]
    fn grouped_row_unions_member_ports() {
        let mut r1 = rowm(1001, "node Helper (a)", 10.0, 0.0);
        r1.ports = vec![3000];
        let mut r2 = rowm(1002, "node Helper (b)", 5.0, 0.0);
        r2.ports = vec![8080, 3000];
        let mut a = App::new();
        a.set_rows(vec![r1, r2]);
        let node = a.display.iter().find(|d| d.label == "node").unwrap();
        assert_eq!(node.ports, vec![3000, 8080], "ports unioned and sorted");
    }

    #[test]
    fn toggle_grouping_splits_into_individual_rows() {
        let mut a = app_with(vec![
            row(1, "Google Chrome Helper (Renderer)", 10.0),
            row(2, "Google Chrome Helper (GPU)", 5.0),
        ]);
        assert_eq!(a.display.len(), 1, "grouped: one Chrome row");
        a.handle_key(KeyCode::Char('t')); // ungroup
        assert_eq!(a.display.len(), 2, "ungrouped: two helper rows");
    }

    #[test]
    fn filter_narrows_to_matching_names() {
        let mut a = app_with(vec![row(1, "python", 9.0), row(2, "node", 8.0)]);
        a.handle_key(KeyCode::Char('/'));
        a.handle_key(KeyCode::Char('p'));
        a.handle_key(KeyCode::Char('y'));
        assert_eq!(a.display.len(), 1);
        assert_eq!(a.display[0].label, "python");
        // Esc clears the filter and restores the full list.
        a.handle_key(KeyCode::Esc);
        assert_eq!(a.display.len(), 2);
    }

    #[test]
    fn sort_by_ram_orders_by_memory() {
        let mut a = app_with(vec![rowm(1, "a", 90.0, 1.0), rowm(2, "b", 1.0, 50.0)]);
        a.handle_key(KeyCode::Char('m'));
        assert_eq!(a.display[0].label, "b", "highest RAM first");
        a.handle_key(KeyCode::Char('c'));
        assert_eq!(a.display[0].label, "a", "highest CPU first");
    }

    #[test]
    fn register_kill_then_disappearance_flips_status_to_killed() {
        let mut a = app_with(vec![row(8981, "python", 99.0), row(1, "a", 1.0)]);
        a.register_kill(vec![8981], "python".to_string());
        assert!(a.status.as_deref().unwrap().contains("killing"));
        a.set_rows(vec![row(1, "a", 1.0)]);
        let s = a.status.as_deref().unwrap();
        assert!(s.contains("killed") && s.contains("python"), "got: {s}");
    }

    #[test]
    fn register_kill_keeps_killing_while_any_pid_present() {
        let mut a = app_with(vec![row(1, "Chrome Helper", 10.0), row(2, "Chrome Helper", 5.0)]);
        a.register_kill(vec![1, 2], "Chrome".to_string());
        // Only pid 1 died; pid 2 still alive → still killing.
        a.set_rows(vec![row(2, "Chrome Helper", 5.0)]);
        assert!(a.status.as_deref().unwrap().contains("killing"));
    }

    #[test]
    fn history_accumulates_cpu_samples_per_row() {
        let mut a = App::new();
        a.set_rows(vec![row(8981, "python", 10.0)]);
        a.set_rows(vec![row(8981, "python", 20.0)]);
        let key = a.display[0].key.clone();
        let hist = a.history.get(&key).unwrap();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist.back().copied(), Some(20.0));
    }

    #[test]
    fn history_drops_rows_that_disappear() {
        let mut a = App::new();
        a.set_rows(vec![row(8981, "python", 10.0), row(8982, "node", 5.0)]);
        a.set_rows(vec![row(8981, "python", 12.0)]);
        // node is gone → its history is dropped.
        assert!(a.history.keys().all(|k| !k.contains("8982")));
        assert_eq!(a.history.len(), 1);
    }

    #[test]
    fn space_marks_and_unmarks_the_selected_row() {
        let mut a = app_with(vec![row(8981, "python", 99.0)]);
        let key = a.display[0].key.clone();
        a.handle_key(KeyCode::Char(' '));
        assert!(a.marked.contains(&key));
        a.handle_key(KeyCode::Char(' '));
        assert!(!a.marked.contains(&key));
    }

    #[test]
    fn k_with_marks_batch_kills_all_marked_killable_rows() {
        let mut a = app_with(vec![row(8981, "python", 99.0), row(8982, "node", 50.0)]);
        a.selected = 0;
        a.handle_key(KeyCode::Char(' ')); // mark python
        a.selected = 1;
        a.handle_key(KeyCode::Char(' ')); // mark node
        match a.handle_key(KeyCode::Char('k')) {
            Action::Kill(mut pids, label) => {
                pids.sort();
                assert_eq!(pids, vec![8981, 8982]);
                assert_eq!(label, "2 selected");
            }
            other => panic!("expected batch Kill, got {other:?}"),
        }
        assert!(a.marked.is_empty(), "marks cleared after kill");
    }

    #[test]
    fn esc_clears_marks_before_quitting() {
        let mut a = app_with(vec![row(8981, "python", 99.0)]);
        a.handle_key(KeyCode::Char(' ')); // mark one
        assert_eq!(a.handle_key(KeyCode::Esc), Action::None, "esc clears marks");
        assert!(a.marked.is_empty());
        assert_eq!(a.handle_key(KeyCode::Esc), Action::Quit, "esc quits when idle");
    }

    #[test]
    fn set_rows_clamps_selection_when_list_shrinks() {
        let mut a = app_with(vec![row(1, "a", 9.0), row(2, "b", 8.0), row(3, "c", 7.0)]);
        a.selected = 2;
        a.set_rows(vec![row(1, "a", 9.0)]);
        assert_eq!(a.selected, 0);
    }
}
