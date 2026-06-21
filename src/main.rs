mod classifier;
mod collector;
mod killer;
mod ui;

use crate::classifier::{classify, Protection};
use crate::collector::Collector;
use crate::ui::{Action, App};
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Row, Table};
use std::io::{self, stdout};
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_millis(1500);

fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let header = Row::new(vec!["PID", "CPU%", "RAM%", "NAME"]).bold();

    let rows: Vec<Row> = app
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let protected = classify(&r.name, r.pid) == Protection::Protected;
            let label = if protected {
                format!("{}  · protected", r.name)
            } else {
                r.name.clone()
            };
            let mut style = if protected {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            if i == app.selected {
                style = style.bg(Color::Blue).add_modifier(Modifier::BOLD);
            }
            Row::new(vec![
                r.pid.to_string(),
                format!("{:.0}", r.cpu_pct),
                format!("{:.1}", r.ram_pct),
                label,
            ])
            .style(style)
        })
        .collect();

    let title = match &app.status {
        Some(s) => format!(" overload — {s} "),
        None => format!(
            " overload — {} procs   [↑↓] move  [k] kill  [q] quit ",
            app.rows.len()
        ),
    };

    let widths = [
        Constraint::Length(7),
        Constraint::Length(6),
        Constraint::Length(6),
        Constraint::Min(10),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title));

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
    let mut last = Instant::now();

    let result = run(&mut terminal, &mut collector, &mut app, &mut last);

    let _ = terminal.show_cursor();
    result
}

fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    collector: &mut Collector,
    app: &mut App,
    last: &mut Instant,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| render(f, app))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == event::KeyEventKind::Press {
                    match app.handle_key(key.code) {
                        Action::Quit => return Ok(()),
                        Action::Kill(pid, name) => {
                            match killer::kill(pid, &name) {
                                Ok(()) => app.status = Some(format!("killing {name} ({pid})…")),
                                Err(e) => app.status = Some(format!("{name}: {e:?}")),
                            }
                        }
                        Action::None => {}
                    }
                }
            }
        }

        if last.elapsed() >= REFRESH {
            let selected_pid = app.rows.get(app.selected).map(|r| r.pid);
            app.set_rows(collector.snapshot());
            // keep cursor on the same pid if it still exists
            if let Some(pid) = selected_pid {
                if let Some(idx) = app.rows.iter().position(|r| r.pid == pid) {
                    app.selected = idx;
                }
            }
            *last = Instant::now();
        }
    }
}
