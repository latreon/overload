mod classifier;
mod collector;
mod killer;
mod ports;
mod ui;

use crate::collector::{Collector, SysStats};
use crate::ui::{Action, App, Sort, ALERT_CPU, HISTORY_LEN};
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
const SPARK_WIDTH: usize = 12;
/// Narrower tables drop the MEM, READ/s, WRITE/s, UPTIME and PEAK columns.
const WIDE_TABLE: u16 = 120;
/// From this width the table also shows STATE and AVG.
const EXTRA_WIDE_TABLE: u16 = 140;
/// Name column width on wide screens, so the detail columns get the space.
const NAME_WIDTH: u16 = 24;
const HELP: &str = " j/↑↓ move · / filter · c m n sort · t group · space mark · p pin · k kill · K kill one · ? help · q quit ";
/// Key bar for narrow terminals, where `HELP` would be cut off.
const SHORT_HELP: &str = " j/↑↓ move · / filter · c m n sort · k kill · ? all keys · q quit ";

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

/// One plain sentence that names what makes the Mac slow right now, if anything.
fn why_slow(app: &App, stats: &SysStats) -> Option<String> {
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
        None
    } else {
        Some(reasons.join(" · "))
    }
}

fn fraction(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        used as f32 / total as f32
    }
}

/// Yellow from 70%, red from 90%, otherwise the accent color.
fn level_color(pct: f32) -> Color {
    if pct >= 90.0 {
        Color::Red
    } else if pct >= 70.0 {
        Color::Yellow
    } else {
        Color::Cyan
    }
}

/// A boxed stat tile with a small caps title.
fn tile(title: String, lines: Vec<Line<'_>>) -> Paragraph<'_> {
    Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(
                format!(" {title} "),
                Style::default().add_modifier(Modifier::BOLD),
            )),
    )
}

fn render(frame: &mut Frame, app: &App, stats: &SysStats) {
    let area = frame.area();
    let dim = Style::default().fg(Color::DarkGray);

    let chunks = Layout::vertical([
        Constraint::Length(4), // stat tiles
        Constraint::Length(1), // why slow + sort state
        Constraint::Min(0),    // process table
    ])
    .split(area);

    let tiles = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(chunks[0]);
    let inner_width = tiles[0].width.saturating_sub(2) as usize;
    let detail_width = inner_width.saturating_sub(5);
    let meter_width = inner_width.saturating_sub(1).min(24);
    let big = |pct: f32| {
        Span::styled(
            format!(" {pct:>3.0}%"),
            Style::default()
                .fg(level_color(pct))
                .add_modifier(Modifier::BOLD),
        )
    };
    let meter = |pct: f32| {
        Span::styled(
            format!(
                " {}",
                bar(pct / 100.0, meter_width)
                    .replace('█', "▰")
                    .replace('░', "▱")
            ),
            Style::default().fg(level_color(pct)),
        )
    };
    let detail = |text: String, style: Style| Span::styled(format!("{text:>detail_width$}"), style);

    let cpu = tile(
        format!("CPU · {} cores", stats.cores),
        vec![
            Line::from(vec![
                big(stats.cpu_pct),
                detail(
                    spark(&app.cpu_history, detail_width.saturating_sub(1)),
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Line::from(meter(stats.cpu_pct)),
        ],
    );
    let memory = tile(
        format!("MEMORY · {}", format_bytes(stats.mem_total)),
        vec![
            Line::from(vec![
                big(stats.mem_pct),
                detail(format!("{} used", format_bytes(stats.mem_used)), dim),
            ]),
            Line::from(meter(stats.mem_pct)),
        ],
    );
    let swap_pct = fraction(stats.swap_used, stats.swap_total) * 100.0;
    let swap = if stats.swap_total == 0 {
        tile("SWAP".to_string(), vec![Line::styled("  off", dim)])
    } else {
        tile(
            format!("SWAP · {}", format_bytes(stats.swap_total)),
            vec![
                Line::from(vec![
                    big(swap_pct),
                    detail(format!("{} used", format_bytes(stats.swap_used)), dim),
                ]),
                Line::from(meter(swap_pct)),
            ],
        )
    };
    let mac = tile(
        format!("MAC · {} procs", stats.proc_count),
        vec![
            Line::from(vec![
                Span::styled(" load ", dim),
                Span::raw(format!(
                    "{:.1} {:.1} {:.1}",
                    stats.load1, stats.load5, stats.load15
                )),
            ]),
            Line::from(vec![
                Span::styled(" up ", dim),
                Span::raw(format_duration(stats.uptime_secs)),
            ]),
        ],
    );
    for (widget, tile_area) in [cpu, memory, swap, mac].into_iter().zip(tiles.iter()) {
        frame.render_widget(widget, *tile_area);
    }

    let why = match why_slow(app, stats) {
        Some(reason) => Line::from(vec![
            Span::styled(" ⚠ ", Style::default().fg(Color::Yellow)),
            Span::styled(reason, Style::default().fg(Color::Yellow)),
        ]),
        None => Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(Color::Green)),
            Span::styled("all calm, nothing heavy right now", dim),
        ]),
    };

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
    // Narrow terminals get a short version so the "why slow" text still fits.
    let is_wide_info = chunks[1].width >= WIDE_TABLE;
    let sorted = if is_wide_info {
        format!(
            "sorted by {} {} ago",
            app.sort.label(),
            format_duration(app.sorted_at.elapsed().as_secs())
        )
    } else {
        format!("by {}", app.sort.label())
    };
    let mut order = vec![Span::styled(sorted, dim)];
    if moved == 0 && new_rows == 0 {
        order.push(Span::styled(" · up to date ", dim));
    } else {
        let accent = Style::default().fg(Color::Yellow);
        if moved > 0 {
            let moved_text = if is_wide_info {
                "out of place"
            } else {
                "moved"
            };
            order.push(Span::styled(format!(" · {moved} {moved_text}"), accent));
        }
        if new_rows > 0 {
            order.push(Span::styled(format!(" · {new_rows} new"), accent));
        }
        order.push(Span::styled(
            if is_wide_info { " · press " } else { " · " },
            dim,
        ));
        order.push(Span::styled(
            format!(" {sort_key} "),
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ));
        order.push(Span::styled(
            if is_wide_info {
                " to sort again "
            } else {
                " sort "
            },
            dim,
        ));
    }
    let order = Line::from(order);
    let info = Layout::horizontal([Constraint::Min(0), Constraint::Length(order.width() as u16)])
        .split(chunks[1]);
    frame.render_widget(Paragraph::new(why), info[0]);
    frame.render_widget(Paragraph::new(order), info[1]);

    let table_area = chunks[2];
    let is_wide = table_area.width >= WIDE_TABLE;
    let is_extra_wide = table_area.width >= EXTRA_WIDE_TABLE;
    let cpu_bar_width = if is_wide { 8 } else { CPU_BAR_WIDTH };

    let mut widths = vec![
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Length(6),
        Constraint::Min(if is_wide { NAME_WIDTH } else { 12 }),
        Constraint::Length(cpu_bar_width as u16 + 5),
        Constraint::Length(6),
    ];
    if is_wide {
        widths.extend([
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(8),
        ]);
    }
    if is_extra_wide {
        widths.extend([Constraint::Length(9), Constraint::Length(4)]);
    }
    if is_wide {
        widths.push(Constraint::Length(5));
    }
    // On wide screens TREND takes the space left after the fixed columns,
    // minus 10% of the width that goes to NAME instead.
    let trend_width = if is_wide {
        let used: u16 = widths
            .iter()
            .map(|width| match width {
                Constraint::Length(cells) | Constraint::Min(cells) => cells + 1,
                _ => 1,
            })
            .sum();
        table_area
            .width
            .saturating_sub(used + 2 + table_area.width / 10)
            .clamp(SPARK_WIDTH as u16, HISTORY_LEN as u16)
    } else {
        SPARK_WIDTH as u16
    };
    widths.push(Constraint::Length(trend_width));

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
        header.extend(["MEM", "READ/s", "WRITE/s", "UPTIME"]);
    }
    if is_extra_wide {
        header.extend(["STATE", "AVG"]);
    }
    if is_wide {
        header.push("PEAK");
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
                bar(d.cpu_pct / 100.0, cpu_bar_width),
                d.cpu_pct
            );
            let history = app.history.get(&d.key);
            let trend = history
                .map(|h| spark(h, trend_width as usize))
                .unwrap_or_default();
            let peak = history
                .and_then(|h| h.iter().copied().reduce(f32::max))
                .unwrap_or(d.cpu_pct);
            let average = history
                .filter(|h| !h.is_empty())
                .map_or(d.cpu_pct, |h| h.iter().sum::<f32>() / h.len() as f32);
            let state_style = match d.state.as_str() {
                "runnable" => Style::default().fg(Color::Green),
                "stopped" => Style::default().fg(Color::Yellow),
                "zombie" => Style::default().fg(Color::Red),
                _ => dim,
            };

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
                    Cell::from(format!("{:>8}", format_bytes(d.read_per_sec))).style(dim),
                    Cell::from(format!("{:>8}", format_bytes(d.write_per_sec))).style(dim),
                    Cell::from(format_duration(d.run_time_secs)).style(dim),
                ]);
            }
            if is_extra_wide {
                cells.extend([
                    Cell::from(d.state.clone()).style(state_style),
                    Cell::from(format!("{average:>4.0}")).style(dim),
                ]);
            }
            if is_wide {
                cells.push(Cell::from(format!("{peak:>5.0}")).style(dim));
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

    // The top border carries the brand, the last action and the view state.
    let mut title = vec![Span::styled(
        " overload ",
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if app.filtering {
        title.push(Span::styled(
            format!(" /{}\u{2588} ", app.filter),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    } else if let Some(status) = &app.status {
        title.push(Span::styled(format!(" {status} "), action_style(status)));
    }
    let mut view_state = Vec::new();
    if !app.filtering && !app.filter.is_empty() {
        view_state.push(format!("filter /{}", app.filter));
    }
    if !app.grouped {
        view_state.push("ungrouped".to_string());
    }
    if !app.marked.is_empty() {
        view_state.push(format!("{} marked", app.marked.len()));
    }
    if !app.pinned.is_empty() {
        view_state.push(format!("{} pinned", app.pinned.len()));
    }
    if !view_state.is_empty() {
        title.push(Span::styled(format!(" {} ", view_state.join(" · ")), dim));
    }
    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(dim)
            .title(Line::from(title))
            .title_bottom(Span::styled(
                if table_area.width as usize > HELP.chars().count() + 2 {
                    HELP
                } else {
                    SHORT_HELP
                },
                dim,
            )),
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
    PortJson(u16),
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
                    if flag != "--port" {
                        Cmd::KillPort(p)
                    } else {
                        match args.get(2).map(String::as_str) {
                            None => Cmd::Port(p),
                            Some("--json") => Cmd::PortJson(p),
                            Some(other) => Cmd::Error(format!("unknown argument: {other}")),
                        }
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
    overload --port <N> --json
                             Same, as a JSON array for scripts
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
        Cmd::PortJson(p) => {
            println!("{}", listeners_json(&ports::on_port(p)));
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

/// Listeners as a JSON array: `[{"pid":1,"command":"node","port":3000}]`.
fn listeners_json(listeners: &[ports::Listener]) -> String {
    let items: Vec<String> = listeners
        .iter()
        .map(|l| {
            format!(
                r#"{{"pid":{},"command":"{}","port":{}}}"#,
                l.pid,
                json_escape(&l.command),
                l.port
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

fn json_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            control if control < ' ' => escaped.push_str(&format!("\\u{:04x}", control as u32)),
            other => escaped.push(other),
        }
    }
    escaped
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
    fn parse_args_maps_port_json() {
        assert_eq!(
            parse_args(&args(&["--port", "3000", "--json"])),
            Cmd::PortJson(3000)
        );
        assert!(matches!(
            parse_args(&args(&["--port", "3000", "--bogus"])),
            Cmd::Error(_)
        ));
    }

    #[test]
    fn listeners_json_escapes_and_handles_empty() {
        assert_eq!(listeners_json(&[]), "[]");
        let listeners = [
            ports::Listener {
                pid: 7,
                command: "no\"de\\x".to_string(),
                port: 3000,
            },
            ports::Listener {
                pid: 8,
                command: "py".to_string(),
                port: 8080,
            },
        ];
        assert_eq!(
            listeners_json(&listeners),
            r#"[{"pid":7,"command":"no\"de\\x","port":3000},{"pid":8,"command":"py","port":8080}]"#
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

    #[test]
    fn render_shows_stat_tiles_and_why_slow() {
        let mut app = App::new();
        app.set_rows(vec![proc(5000, "node", 95.0, 1.0)]);
        let text = rendered_text(&app, 100, 12);
        for tile in ["CPU", "MEMORY", "SWAP", "MAC"] {
            assert!(text.contains(tile), "{tile} tile missing");
        }
        assert!(text.contains("node uses 95% CPU"), "why slow line missing");
    }

    #[test]
    fn key_bar_is_short_on_narrow_terminals() {
        let mut app = App::new();
        app.set_rows(vec![proc(5000, "node", 10.0, 1.0)]);
        assert!(rendered_text(&app, 80, 12).contains("? all keys"));
        assert!(rendered_text(&app, 160, 12).contains("K kill one"));
    }

    #[test]
    fn format_helpers_use_short_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(520 * 1024 * 1024), "520 MB");
        assert_eq!(format_bytes(2_576_980_378), "2.4 GB");
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(38 * 60), "38m");
        assert_eq!(format_duration(6 * 3_600 + 12 * 60), "6h 12m");
        assert_eq!(format_duration(2 * 86_400 + 4 * 3_600), "2d 4h");
    }
}
