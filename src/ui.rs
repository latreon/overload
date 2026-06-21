use crate::classifier::{classify, Protection};
use crate::collector::ProcRow;
use crossterm::event::KeyCode;

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Kill(u32),
}

pub struct App {
    pub rows: Vec<ProcRow>,
    pub selected: usize,
    pub status: Option<String>,
}

impl App {
    pub fn new() -> Self {
        App { rows: Vec::new(), selected: 0, status: None }
    }

    pub fn set_rows(&mut self, rows: Vec<ProcRow>) {
        self.rows = rows;
        let max = self.rows.len().saturating_sub(1);
        if self.selected > max {
            self.selected = max;
        }
    }

    pub fn handle_key(&mut self, code: KeyCode) -> Action {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
            KeyCode::Down => {
                if self.selected + 1 < self.rows.len() {
                    self.selected += 1;
                }
                Action::None
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                Action::None
            }
            KeyCode::Char('k') => {
                let Some(r) = self.rows.get(self.selected) else {
                    return Action::None;
                };
                match classify(&r.name, r.pid) {
                    Protection::Protected => {
                        self.status = Some(format!("{} is protected — not killed", r.name));
                        Action::None
                    }
                    Protection::Killable => Action::Kill(r.pid),
                }
            }
            _ => Action::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, name: &str, cpu: f32) -> ProcRow {
        ProcRow { pid, name: name.to_string(), cpu_pct: cpu, ram_pct: 0.0 }
    }

    fn app_with(rows: Vec<ProcRow>) -> App {
        let mut a = App::new();
        a.set_rows(rows);
        a
    }

    #[test]
    fn down_and_up_move_selection_within_bounds() {
        let mut a = app_with(vec![row(1, "a", 9.0), row(2, "b", 8.0)]);
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
    fn q_returns_quit() {
        let mut a = app_with(vec![row(500, "x", 1.0)]);
        assert_eq!(a.handle_key(KeyCode::Char('q')), Action::Quit);
    }

    #[test]
    fn k_on_killable_returns_kill_with_pid() {
        let mut a = app_with(vec![row(8981, "python", 99.0)]);
        assert_eq!(a.handle_key(KeyCode::Char('k')), Action::Kill(8981));
    }

    #[test]
    fn k_on_protected_returns_none_and_sets_status() {
        let mut a = app_with(vec![row(50, "WindowServer", 42.0)]);
        assert_eq!(a.handle_key(KeyCode::Char('k')), Action::None);
        assert!(a.status.is_some(), "should explain why nothing happened");
    }

    #[test]
    fn set_rows_clamps_selection_when_list_shrinks() {
        let mut a = app_with(vec![row(1, "a", 9.0), row(2, "b", 8.0), row(3, "c", 7.0)]);
        a.selected = 2;
        a.set_rows(vec![row(1, "a", 9.0)]);
        assert_eq!(a.selected, 0);
    }
}
