mod classifier;
mod collector;
mod killer;
mod ports;
mod ui;

use crate::collector::{Collector, SysStats};
use crate::ui::{Action, App, Sort, ALERT_CPU};
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table};
use std::collections::VecDeque;
use std::io::{self, stdout};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_millis(1500);
const CPU_BAR_WIDTH: usize = 6;
const GAUGE_WIDTH: usize = 16;
const SPARK_WIDTH: usize = 12;
const CPU_HISTORY_WIDTH: usize = 30;
/// Narrower tables drop the MEM, DISK/s, UPTIME and PEAK columns.
const WIDE_TABLE: u16 = 120;
const HELP: &str = " j/↑↓ move · / filter · c m n sort · t group · space mark · p pin · k kill · K kill one · ? help · q quit ";

/// Levels for sparkline / bar glyphs, low → high.
const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Style the persistent action line by what it reports.
fn action_style(status: &str) -> Style {
    let base = if status.contains("killed") {
        Style::default().fg(Color::Green)
    } else if status.contains("killing") {
        Style::default().fg(Color::Yellow)
    } else {
        // protected / permission denied / other errors
        Style::default().fg(Color::Red)
    };
    base.add_modifier(Modifier::BOLD)
}

/// Smooth green→yellow→red gradient for a 0..=100 percentage.
fn gradient(pct: f32) -> Color {
    let t = (pct / 100.0).clamp(0.0, 1.0);
    let lerp = |a: f32, b: f32, u: f32| (a + (b - a) * u) as u8;
    // green (46,204,113) → yellow (241,196,15) → red (231,76,60)
    if t < 0.5 {
        let u = t * 2.0;
        Color::Rgb(
            lerp(46.0, 241.0, u),
            lerp(204.0, 196.0, u),
            lerp(113.0, 15.0, u),
        )
    } else {
        let u = (t - 0.5) * 2.0;
        Color::Rgb(
            lerp(241.0, 231.0, u),
            lerp(196.0, 76.0, u),
            lerp(15.0, 60.0, u),
        )
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

/// Render the last `width` CPU samples as a block sparkline (scaled to one
/// full core = 100%). Left-padded with spaces when there are fewer samples.
fn spark(samples: &VecDeque<f32>, width: usize) -> String {
    let mut s = String::with_capacity(width);
    let n = samples.len();
    let start = n.saturating_sub(width);
    for _ in 0..width.saturating_sub(n) {
        s.push(' ');
    }
    for v in samples.iter().skip(start) {
        let lvl = ((v / 100.0).clamp(0.0, 1.0) * (BLOCKS.len() - 1) as f32).round() as usize;
        s.push(BLOCKS[lvl]);
    }
    s
}

/// "2.4 GB", "520 MB", "12 KB".
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == UNITS.len() - 1 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// "2d 4h", "6h 12m", "38m", "45s".
fn format_duration(secs: u64) -> String {
    let (days, hours, minutes) = (secs / 86_400, secs / 3_600 % 24, secs / 60 % 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{secs}s")
    }
}

/// One plain sentence that names what makes the Mac slow right now.
fn why_slow(app: &App, stats: &SysStats) -> String {
    let mut reasons = Vec::new();
    let busiest = app
        .display
        .iter()
        .filter(|d| !d.protected)
        .max_by(|a, b| a.cpu_pct.total_cmp(&b.cpu_pct));
    if let Some(d) = busiest.filter(|d| d.cpu_pct >= 25.0) {
        reasons.push(format!("{} uses {:.0}% CPU", d.label, d.cpu_pct));
    }
    if stats.mem_pct >= 80.0 {
        reasons.push(format!("memory {:.0}% full", stats.mem_pct));
    }
    if stats.swap_used >= 1 << 30 {
        reasons.push(format!("swap {} in use", format_bytes(stats.swap_used)));
    }
    if reasons.is_empty() {
        "all calm, nothing heavy right now".to_string()
    } else {
        reasons.join(" · ")
    }
}

fn fraction(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        used as f32 / total as f32
    }
}

fn render(frame: &mut Frame, app: &App, stats: &SysStats) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(1), // action / status
        Constraint::Length(1), // CPU, MEM and SWAP gauges
        Constraint::Length(1), // CPU history and Mac summary
        Constraint::Length(1), // sort and order state
        Constraint::Length(1), // why slow
        Constraint::Min(0),    // process table
    ])
    .split(area);

    // Action line. While typing a filter it shows the live query; otherwise
    // it persists the last action (killing…/✓ killed/errors) or a hint.
    let action = if app.filtering {
        Paragraph::new(format!("  /{}\u{2588}", app.filter)).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        match &app.status {
            Some(s) => Paragraph::new(format!("  {s}")).style(action_style(s)),
            None => Paragraph::new("  select a process and press [k] to kill it")
                .style(Style::default().fg(Color::DarkGray)),
        }
    };
    frame.render_widget(action, chunks[0]);

    let dim = Style::default().fg(Color::DarkGray);
    let swap_fraction = fraction(stats.swap_used, stats.swap_total);
    let gauges = Line::from(vec![
        Span::styled(" CPU ", dim),
        Span::styled(
            bar(stats.cpu_pct / 100.0, GAUGE_WIDTH),
            Style::default().fg(gradient(stats.cpu_pct)),
        ),
        Span::raw(format!(" {:>3.0}%    ", stats.cpu_pct)),
        Span::styled("MEM ", dim),
        Span::styled(
            bar(stats.mem_pct / 100.0, GAUGE_WIDTH),
            Style::default().fg(gradient(stats.mem_pct)),
        ),
        Span::raw(format!(
            " {} / {}    ",
            format_bytes(stats.mem_used),
            format_bytes(stats.mem_total)
        )),
        Span::styled("SWAP ", dim),
        Span::styled(
            bar(swap_fraction, GAUGE_WIDTH / 2),
            Style::default().fg(gradient(swap_fraction * 100.0)),
        ),
        Span::raw(format!(
            " {} / {}",
            format_bytes(stats.swap_used),
            format_bytes(stats.swap_total)
        )),
    ]);
    frame.render_widget(Paragraph::new(gauges), chunks[1]);

    let summary = Line::from(vec![
        Span::styled(" overload ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            spark(&app.cpu_history, CPU_HISTORY_WIDTH),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled("   load ", dim),
        Span::raw(format!(
            "{:.2} {:.2} {:.2}",
            stats.load1, stats.load5, stats.load15
        )),
        Span::styled("   up ", dim),
        Span::raw(format_duration(stats.uptime_secs)),
        Span::styled("   cores ", dim),
        Span::raw(stats.cores.to_string()),
        Span::styled("   procs ", dim),
        Span::raw(stats.proc_count.to_string()),
    ]);
    frame.render_widget(Paragraph::new(summary), chunks[2]);

    let moved = app.display.iter().filter(|d| d.moves != 0).count();
    let new_rows = app
        .display
        .iter()
        .filter(|d| !app.sorted_keys.contains(&d.key))
        .count();
    let sort_key = match app.sort {
        Sort::Cpu => 'c',
        Sort::Ram => 'm',
        Sort::Name => 'n',
    };
    let order = if moved == 0 && new_rows == 0 {
        Span::styled("order matches the sort", Style::default().fg(Color::Green))
    } else {
        Span::styled(
            format!(
                "order kept {} · {moved} would move · {new_rows} new · {sort_key} to re-sort",
                format_duration(app.sorted_at.elapsed().as_secs())
            ),
            Style::default().fg(Color::Yellow),
        )
    };
    let mut order_line = vec![
        Span::styled(" sort ", dim),
        Span::styled(
            format!(" {} ", app.sort.label()),
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
        Span::raw("  "),
        order,
        Span::styled(
            format!("   group {}", if app.grouped { "on" } else { "off" }),
            dim,
        ),
    ];
    if !app.filter.is_empty() {
        order_line.push(Span::styled(format!("   filter /{}", app.filter), dim));
    }
    if !app.marked.is_empty() {
        order_line.push(Span::styled(format!("   {} marked", app.marked.len()), dim));
    }
    if !app.pinned.is_empty() {
        order_line.push(Span::styled(format!("   {} pinned", app.pinned.len()), dim));
    }
    frame.render_widget(Paragraph::new(Line::from(order_line)), chunks[3]);

    let why = Line::from(vec![
        Span::styled(" why slow? ", dim),
        Span::styled(why_slow(app, stats), Style::default().fg(Color::Yellow)),
    ]);
    frame.render_widget(Paragraph::new(why), chunks[4]);

    let table_area = chunks[5];
    let is_wide = table_area.width >= WIDE_TABLE;

    // Pin the selected row near the vertical center; the list scrolls around it.
    let view_h = (table_area.height.saturating_sub(3)).max(1) as usize; // borders + header
    let len = app.display.len();
    let offset = if len <= view_h {
        0
    } else {
        app.selected.saturating_sub(view_h / 2).min(len - view_h)
    };
    let end = (offset + view_h).min(len);

    let mut header = vec![" ", "MOVE", "PID", "NAME", "CPU", "RAM"];
    if is_wide {
        header.extend(["MEM", "DISK/s", "UPTIME", "PEAK"]);
    }
    header.push("TREND");
    let header = Row::new(header)
        .style(Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED));

    let rows: Vec<Row> = app.display[offset..end]
        .iter()
        .enumerate()
        .map(|(vi, d)| {
            let idx = offset + vi;
            let marked = app.marked.contains(&d.key);

            // Gutter marker: ● marked, ▸ selected, else blank.
            let marker = if marked {
                "●"
            } else if idx == app.selected {
                "▸"
            } else {
                " "
            };

            // pid for a single process, or ×N for a group.
            let id = if d.count > 1 {
                format!("×{:<4}", d.count)
            } else {
                format!("{:>5}", d.pids.first().copied().unwrap_or(0))
            };
            let ports = if d.ports.is_empty() {
                String::new()
            } else {
                let list = d
                    .ports
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(" :");
                format!("  :{list}")
            };
            let is_alerted = app.is_alerted(&d.key);
            let label_style = if d.protected {
                dim
            } else if is_alerted {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let mut name = Vec::new();
            if app.pinned.contains(&d.key) {
                name.push(Span::styled("◆ ", Style::default().fg(Color::Yellow)));
            }
            if is_alerted {
                name.push(Span::styled("⚠ ", label_style));
            }
            name.push(Span::styled(d.label.clone(), label_style));
            name.push(Span::styled(ports, Style::default().fg(Color::Cyan)));
            if d.protected {
                name.push(Span::styled(" · protected", dim));
            }
            if !app.sorted_keys.contains(&d.key) {
                name.push(Span::raw(" "));
                name.push(Span::styled(
                    " NEW ",
                    Style::default().fg(Color::Black).bg(Color::Cyan),
                ));
            }

            let move_cell = match d.moves {
                0 => Cell::from("·").style(dim),
                up if up > 0 => {
                    Cell::from(format!("↑{up}")).style(Style::default().fg(Color::Green))
                }
                down => {
                    Cell::from(format!("↓{}", -down)).style(Style::default().fg(Color::LightRed))
                }
            };

            let cpu_txt = format!(
                "{} {:>3.0}",
                bar(d.cpu_pct / 100.0, CPU_BAR_WIDTH),
                d.cpu_pct
            );
            let history = app.history.get(&d.key);
            let trend = history.map(|h| spark(h, SPARK_WIDTH)).unwrap_or_default();
            let peak = history
                .and_then(|h| h.iter().copied().reduce(f32::max))
                .unwrap_or(d.cpu_pct);

            let (cpu_style, ram_style, trend_style) = if d.protected {
                (dim, dim, dim)
            } else {
                (
                    Style::default().fg(gradient(d.cpu_pct)),
                    Style::default().fg(gradient(d.ram_pct)),
                    Style::default().fg(Color::Cyan),
                )
            };

            let marker_cell = Cell::from(marker).style(Style::default().fg(if marked {
                Color::Cyan
            } else {
                Color::Blue
            }));

            let mut cells = vec![
                marker_cell,
                move_cell,
                Cell::from(id).style(dim),
                Cell::from(Line::from(name)),
                Cell::from(cpu_txt).style(cpu_style),
                Cell::from(format!("{:>4.1}%", d.ram_pct)).style(ram_style),
            ];
            if is_wide {
                cells.extend([
                    Cell::from(format!("{:>8}", format_bytes(d.mem_bytes))),
                    Cell::from(format!("{:>8}", format_bytes(d.disk_per_sec))).style(dim),
                    Cell::from(format_duration(d.run_time_secs)).style(dim),
                    Cell::from(format!("{peak:>4.0}")).style(dim),
                ]);
            }
            cells.push(Cell::from(trend).style(trend_style));
            let mut row = Row::new(cells);
            if idx == app.selected {
                row = row.style(
                    Style::default()
                        .bg(Color::Indexed(238))
                        .add_modifier(Modifier::BOLD),
                );
            }
            row
        })
        .collect();

    let mut widths = vec![
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Length(6),
        Constraint::Min(12),
        Constraint::Length(CPU_BAR_WIDTH as u16 + 5),
        Constraint::Length(6),
    ];
    if is_wide {
        widths.extend([
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Length(4),
        ]);
    }
    widths.push(Constraint::Length(SPARK_WIDTH as u16));
    let marks = app.marked.len();
    let title = if marks > 0 {
        format!(" processes — {marks} marked ")
    } else {
        " processes ".to_string()
    };
    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(title)
            .title_bottom(HELP),
    );

    frame.render_widget(table, table_area);

    if app.show_help {
        render_help(frame, app, area);
    }
}

/// Centered popup with every key and the recent kills.
fn render_help(frame: &mut Frame, app: &App, area: Rect) {
    let keys = [
        ("j / ↓  ↑", "move"),
        ("g / G", "jump to top / bottom"),
        ("/", "filter by name"),
        ("c m n", "sort by CPU, memory, name"),
        ("t", "group helpers on or off"),
        ("space", "mark a row, Esc clears marks"),
        ("p", "pin the row on top"),
        ("k", "kill the row or all marked rows"),
        ("K", "kill only the busiest process of the row"),
        ("?", "show this help"),
        ("q", "quit"),
    ];
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::DarkGray);
    let mut lines: Vec<Line> = keys
        .iter()
        .map(|(key_names, text)| {
            Line::from(vec![
                Span::styled(format!("  {key_names:<10}"), bold.fg(Color::Cyan)),
                Span::raw(*text),
            ])
        })
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::styled("  Recent kills", bold));
    if app.kill_log.is_empty() {
        lines.push(Line::styled("  none yet", dim));
    }
    for (killed_at, label) in app.kill_log.iter().rev() {
        lines.push(Line::raw(format!(
            "  {:>7} ago  {label}",
            format_duration(killed_at.elapsed().as_secs())
        )));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "  Rows stay in place until you sort again.",
        dim,
    ));
    lines.push(Line::styled(
        format!("  Rows over {ALERT_CPU:.0}% CPU for 30s turn red and send a notice."),
        dim,
    ));
    lines.push(Line::styled("  Press any key to close.", dim));

    let width = 66.min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" help "),
        ),
        popup,
    );
}

/// Show a macOS notice. The text goes in as an argument, never into the script.
fn notify(label: &str) {
    let message = format!("{label} over {ALERT_CPU:.0}% CPU for 30s");
    let child = Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 1 of argv) with title \"overload\"",
            "-e",
            "end run",
            &message,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        std::thread::spawn(move || child.wait());
    }
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

/// Parsed command line. With no recognized flag, launches the TUI.
#[derive(Debug, PartialEq, Eq)]
enum Cmd {
    Tui,
    Help,
    Port(u16),
    KillPort(u16),
    Error(String),
}

fn parse_args(args: &[String]) -> Cmd {
    match args.first().map(String::as_str) {
        None => Cmd::Tui,
        Some("-h") | Some("--help") => Cmd::Help,
        Some("--port") | Some("--kill-port") => {
            let flag = &args[0];
            let Some(raw) = args.get(1) else {
                return Cmd::Error(format!("{flag} needs a port number"));
            };
            match raw.parse::<u16>() {
                Ok(p) if p > 0 => {
                    if flag == "--port" {
                        Cmd::Port(p)
                    } else {
                        Cmd::KillPort(p)
                    }
                }
                _ => Cmd::Error(format!("invalid port: {raw}")),
            }
        }
        Some(other) => Cmd::Error(format!("unknown argument: {other}")),
    }
}

const USAGE: &str = "overload — find and kill resource-hungry processes

USAGE:
    overload                 Launch the interactive TUI
    overload --port <N>      Show what is listening on TCP port N
    overload --kill-port <N> Kill whatever is listening on TCP port N
    overload --help          Show this help

TUI KEYS:
    j / ↑ ↓   move      g / G   top / bottom     / filter
    c m n     sort      t       group helpers    space mark
    p         pin       k       kill             K kill busiest only
    ?         help      q       quit";

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Cmd::Help => {
            println!("{USAGE}");
            Ok(())
        }
        Cmd::Port(p) => {
            cmd_port(p);
            Ok(())
        }
        Cmd::KillPort(p) => {
            cmd_kill_port(p);
            Ok(())
        }
        Cmd::Error(msg) => {
            eprintln!("overload: {msg}\n\n{USAGE}");
            std::process::exit(2);
        }
        Cmd::Tui => run_tui(),
    }
}

/// `--port N`: print the processes listening on a port.
fn cmd_port(port: u16) {
    let listeners = ports::on_port(port);
    if listeners.is_empty() {
        println!("nothing is listening on port {port}");
        return;
    }
    println!("port {port}:");
    for l in listeners {
        println!("  {:>7}  {}", l.pid, l.command);
    }
}

/// `--kill-port N`: kill whatever is listening on a port.
fn cmd_kill_port(port: u16) {
    let listeners = ports::on_port(port);
    if listeners.is_empty() {
        println!("nothing is listening on port {port}");
        return;
    }
    for l in listeners {
        match killer::kill(l.pid, &l.command) {
            Ok(()) => println!("killed {} ({}) on port {port}", l.command, l.pid),
            Err(e) => println!("could not kill {} ({}): {e:?}", l.command, l.pid),
        }
    }
}

/// Take a process snapshot and annotate each row with its listening ports.
fn snapshot_with_ports(collector: &mut Collector) -> Vec<collector::ProcRow> {
    let mut rows = collector.snapshot();
    let port_map = ports::ports_by_pid();
    for r in &mut rows {
        if let Some(p) = port_map.get(&r.pid) {
            r.ports = p.clone();
        }
    }
    rows
}

fn run_tui() -> io::Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    // Guard owns raw-mode + alternate-screen restoration from this point on.
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    let mut collector = Collector::new();
    let mut app = App::new();

    // sysinfo reports 0% CPU on the first read (it needs two samples spaced
    // apart to compute deltas). Prime once, wait a beat, then take the real
    // snapshot so the first frame is correctly sorted with the cursor on the
    // hottest process — no need to scroll up after launch.
    let _ = collector.snapshot();
    let _ = collector.system_stats();
    std::thread::sleep(Duration::from_millis(250));
    app.set_rows(snapshot_with_ports(&mut collector));
    let mut stats = collector.system_stats();
    app.record_cpu(stats.cpu_pct);
    let mut last = Instant::now();

    let result = run(
        &mut terminal,
        &mut collector,
        &mut app,
        &mut stats,
        &mut last,
    );

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
                        Action::Kill(pids, label) => {
                            // Kill every pid in the row; register those accepted.
                            let mut killed = Vec::new();
                            let mut last_err = None;
                            for pid in &pids {
                                match killer::kill(*pid, &label) {
                                    Ok(()) => killed.push(*pid),
                                    Err(e) => last_err = Some(e),
                                }
                            }
                            if killed.is_empty() {
                                app.status = Some(format!("{label}: {last_err:?}"));
                            } else {
                                app.register_kill(killed, label);
                            }
                        }
                        Action::None => {}
                    }
                }
            }
        }

        if last.elapsed() >= REFRESH {
            // set_rows rebuilds the view and keeps the cursor on the same row.
            app.set_rows(snapshot_with_ports(collector));
            for label in app.new_alerts.drain(..) {
                notify(&label);
            }
            *stats = collector.system_stats();
            app.record_cpu(stats.cpu_pct);
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
        SysStats {
            load1: 5.2,
            cpu_pct: 78.0,
            mem_pct: 61.0,
            proc_count: 3,
            ..Default::default()
        }
    }

    fn proc(pid: u32, name: &str, cpu: f32, ram: f32) -> ProcRow {
        ProcRow {
            pid,
            name: name.to_string(),
            cpu_pct: cpu,
            ram_pct: ram,
            ..Default::default()
        }
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

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_args_maps_flags() {
        assert_eq!(parse_args(&args(&[])), Cmd::Tui);
        assert_eq!(parse_args(&args(&["--help"])), Cmd::Help);
        assert_eq!(parse_args(&args(&["-h"])), Cmd::Help);
        assert_eq!(parse_args(&args(&["--port", "3000"])), Cmd::Port(3000));
        assert_eq!(
            parse_args(&args(&["--kill-port", "8080"])),
            Cmd::KillPort(8080)
        );
    }

    #[test]
    fn parse_args_reports_errors() {
        assert!(matches!(parse_args(&args(&["--port"])), Cmd::Error(_)));
        assert!(matches!(parse_args(&args(&["--port", "0"])), Cmd::Error(_)));
        assert!(matches!(
            parse_args(&args(&["--port", "notnum"])),
            Cmd::Error(_)
        ));
        assert!(matches!(parse_args(&args(&["--bogus"])), Cmd::Error(_)));
    }

    #[test]
    fn render_shows_trend_column_and_mark() {
        let mut app = App::new();
        app.set_rows(vec![proc(8981, "python", 10.0, 1.0)]);
        let key = app.display[0].key.clone();
        app.marked.insert(key);
        let text = rendered_text(&app, 80, 9);
        assert!(text.contains("TREND"), "trend column header missing");
        assert!(text.contains('●'), "marked-row bullet missing");
    }

    #[test]
    fn render_shows_listening_port() {
        let mut app = App::new();
        let mut p = proc(8981, "node", 30.0, 1.0);
        p.ports = vec![3000];
        app.set_rows(vec![p]);
        let text = rendered_text(&app, 80, 10);
        assert!(text.contains(":3000"), "listening port should be shown");
    }

    #[test]
    fn bar_fills_proportionally_and_clamps() {
        assert_eq!(bar(0.0, 4), "░░░░");
        assert_eq!(bar(1.0, 4), "████");
        assert_eq!(bar(1.5, 4), "████"); // over 100% is clamped
        assert_eq!(bar(0.5, 4), "██░░");
    }

    #[test]
    fn gradient_runs_green_to_red() {
        // Endpoints land on the defined anchor colors; midpoint is yellow-ish.
        assert_eq!(gradient(0.0), Color::Rgb(46, 204, 113));
        assert_eq!(gradient(100.0), Color::Rgb(231, 76, 60));
        assert_eq!(gradient(50.0), Color::Rgb(241, 196, 15));
    }

    #[test]
    fn spark_scales_and_left_pads() {
        let mut h = VecDeque::new();
        h.push_back(0.0);
        h.push_back(100.0);
        let s = spark(&h, 4);
        // 4 wide, 2 samples → 2 leading spaces, then lowest + highest block.
        assert_eq!(s.chars().count(), 4);
        assert!(s.starts_with("  "), "left-padded: {s:?}");
        assert!(s.ends_with('█'), "100% → full block: {s:?}");
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
        let target_pid = app.display[80].pids[0].to_string();
        let text = rendered_text(&app, 80, 10);
        assert!(
            text.contains(&target_pid),
            "selected row (pid {target_pid}) should be scrolled into view"
        );
    }
}
