mod classifier;
mod collector;
mod killer;
mod ui;

use crate::classifier::{classify, Protection};
use crate::collector::{Collector, SysStats};
use crate::ui::{Action, App};
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use std::io::{self, stdout};
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_millis(1500);
const CPU_BAR_WIDTH: usize = 6;
const RAM_BAR_WIDTH: usize = 4;
const HELP: &str = " [↑↓] move   [k] kill   [q] quit ";

/// Color a percentage by intensity: green (calm) → yellow → red (hot).
fn heat(pct: f32) -> Color {
    if pct >= 70.0 {
        Color::Red
    } else if pct >= 30.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

/// A unicode meter: `frac` (0..=1) of `width` cells filled.
fn bar(frac: f32, width: usize) -> String {
    let filled = (frac.clamp(0.0, 1.0) * width as f32).round() as usize;
    let filled = filled.min(width);
    let mut s = String::with_capacity(width * 3);
    s.extend(std::iter::repeat_n('█', filled));
    s.extend(std::iter::repeat_n('░', width - filled));
    s
}

fn render(frame: &mut Frame, app: &App, stats: &SysStats) {
    let area = frame.area();

    // Pin the selected row near the vertical center; the list scrolls around it.
    let view_h = (area.height.saturating_sub(3)).max(1) as usize; // borders + header
    let len = app.rows.len();
    let offset = if len <= view_h {
        0
    } else {
        app.selected
            .saturating_sub(view_h / 2)
            .min(len - view_h)
    };
    let end = (offset + view_h).min(len);

    let header = Row::new(vec!["PID", "CPU", "RAM", "NAME"])
        .style(Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED));

    let rows: Vec<Row> = app.rows[offset..end]
        .iter()
        .enumerate()
        .map(|(vi, r)| {
            let idx = offset + vi;
            let protected = classify(&r.name, r.pid) == Protection::Protected;

            let pid_cell = Cell::from(format!("{:>6}", r.pid));
            let (cpu_cell, ram_cell, name_cell) = if protected {
                let grey = Style::default().fg(Color::DarkGray);
                (
                    Cell::from(format!("{} {:>3.0}", bar(r.cpu_pct / 100.0, CPU_BAR_WIDTH), r.cpu_pct))
                        .style(grey),
                    Cell::from(format!("{} {:>4.1}", bar(r.ram_pct / 100.0, RAM_BAR_WIDTH), r.ram_pct))
                        .style(grey),
                    Cell::from(format!("{} · protected", r.name)).style(grey),
                )
            } else {
                (
                    Cell::from(format!("{} {:>3.0}", bar(r.cpu_pct / 100.0, CPU_BAR_WIDTH), r.cpu_pct))
                        .style(Style::default().fg(heat(r.cpu_pct))),
                    Cell::from(format!("{} {:>4.1}", bar(r.ram_pct / 100.0, RAM_BAR_WIDTH), r.ram_pct))
                        .style(Style::default().fg(heat(r.ram_pct))),
                    Cell::from(r.name.clone()),
                )
            };

            let mut row = Row::new(vec![pid_cell, cpu_cell, ram_cell, name_cell]);
            if idx == app.selected {
                row = row.style(
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                );
            }
            row
        })
        .collect();

    let top = format!(
        " overload   load {:.2}   cpu {:.0}%   mem {:.0}%   {} procs ",
        stats.load1, stats.cpu_pct, stats.mem_pct, stats.proc_count
    );
    let bottom = app.status.clone().unwrap_or_else(|| HELP.to_string());

    let widths = [
        Constraint::Length(6),
        Constraint::Length(CPU_BAR_WIDTH as u16 + 5),
        Constraint::Length(RAM_BAR_WIDTH as u16 + 6),
        Constraint::Min(8),
    ];
    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .borders(Borders::ALL)
            .title(top)
            .title_bottom(bottom),
    );

    frame.render_widget(table, area);
}

/// RAII guard that restores the terminal on every exit path — including panics.
/// Raw mode and the alternate screen are both restored in `drop`, so even if
/// `run` panics the terminal is left in a usable state.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn main() -> io::Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    // Guard owns raw-mode + alternate-screen restoration from this point on.
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let mut collector = Collector::new();
    let mut app = App::new();
    app.set_rows(collector.snapshot()); // prime
    let mut stats = collector.system_stats(); // prime
    let mut last = Instant::now();

    let result = run(&mut terminal, &mut collector, &mut app, &mut stats, &mut last);

    let _ = terminal.show_cursor();
    result
}

fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    collector: &mut Collector,
    app: &mut App,
    stats: &mut SysStats,
    last: &mut Instant,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| render(f, app, stats))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == event::KeyEventKind::Press {
                    match app.handle_key(key.code) {
                        Action::Quit => return Ok(()),
                        Action::Kill(pid, name) => match killer::kill(pid, &name) {
                            Ok(()) => app.register_kill(pid, name),
                            Err(e) => app.status = Some(format!("{name}: {e:?}")),
                        },
                        Action::None => {}
                    }
                }
            }
        }

        if last.elapsed() >= REFRESH {
            let selected_pid = app.rows.get(app.selected).map(|r| r.pid);
            app.set_rows(collector.snapshot());
            *stats = collector.system_stats();
            // keep the cursor on the same process if it still exists
            if let Some(pid) = selected_pid {
                if let Some(idx) = app.rows.iter().position(|r| r.pid == pid) {
                    app.selected = idx;
                }
            }
            *last = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::ProcRow;
    use ratatui::backend::TestBackend;

    fn sample_stats() -> SysStats {
        SysStats { load1: 5.2, cpu_pct: 78.0, mem_pct: 61.0, proc_count: 3 }
    }

    fn proc(pid: u32, name: &str, cpu: f32, ram: f32) -> ProcRow {
        ProcRow { pid, name: name.to_string(), cpu_pct: cpu, ram_pct: ram }
    }

    fn rendered_text(app: &App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let stats = sample_stats();
        terminal.draw(|f| render(f, app, &stats)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn bar_fills_proportionally_and_clamps() {
        assert_eq!(bar(0.0, 4), "░░░░");
        assert_eq!(bar(1.0, 4), "████");
        assert_eq!(bar(1.5, 4), "████"); // over 100% is clamped
        assert_eq!(bar(0.5, 4), "██░░");
    }

    #[test]
    fn heat_escalates_with_intensity() {
        assert_eq!(heat(10.0), Color::Green);
        assert_eq!(heat(50.0), Color::Yellow);
        assert_eq!(heat(95.0), Color::Red);
    }

    #[test]
    fn render_shows_header_rows_and_protected_flag() {
        let mut app = App::new();
        app.set_rows(vec![
            proc(2344, "Dia", 130.0, 1.2),
            proc(8981, "python", 99.0, 2.0),
            proc(50, "WindowServer", 42.0, 0.6),
        ]);
        let text = rendered_text(&app, 80, 12);
        assert!(text.contains("overload"), "header missing");
        assert!(text.contains("2344"), "process pid missing");
        assert!(text.contains("protected"), "protected flag missing");
    }

    #[test]
    fn render_pins_selected_into_view_for_long_lists() {
        // 100 rows, tiny viewport, selection deep in the list: the selected
        // pid must still be drawn (proves the centered-scroll offset works).
        let mut rows = Vec::new();
        for i in 0..100u32 {
            rows.push(proc(1000 + i, &format!("p{i}"), (100 - i) as f32, 0.1));
        }
        let mut app = App::new();
        app.set_rows(rows);
        app.selected = 80;
        let target_pid = app.rows[80].pid.to_string();
        let text = rendered_text(&app, 80, 10);
        assert!(
            text.contains(&target_pid),
            "selected row (pid {target_pid}) should be scrolled into view"
        );
    }
}
