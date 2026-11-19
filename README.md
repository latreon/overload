# overload

A macOS terminal tool to find and kill resource hogs fast.

## Usage

```
overload                 # launch the interactive TUI
overload --port 3000     # show what is listening on TCP port 3000
overload --kill-port 3000  # kill whatever is listening on port 3000
overload --help          # usage
```

- `j` / `↑` `↓` — move selection
- `g` / `G` (or Home/End) — jump to top / bottom; `PgUp` / `PgDn` — page
- `/` — filter by name (type to narrow, `Esc` clears, `Enter` keeps it)
- `c` / `m` / `n` — sort by CPU / memory / name
- `t` — toggle grouping of an app's helper processes
- `space` — mark/unmark a row for batch kill (`Esc` clears marks)
- `p` — pin/unpin the selected row on top
- `k` — kill the selected row, or all marked rows (SIGTERM, then SIGKILL after 2s)
- `K` — kill only the busiest process of the selected row
- `?` — help, with the recent kills
- `q` — quit

**Live trends & gauges.** Four tiles sit at the top: CPU (with recent history),
memory, swap and the Mac (load for 1, 5, 15 min, uptime, process count). Numbers
turn yellow at 70% and red at 90%. Under the tiles, one line names the problem
in plain words, for example `⚠ node uses 95% CPU · swap 1.2 GB in use`, and
says if the list is out of order. Every row carries a `TREND` sparkline
of its own recent CPU history. Wide terminals (120+ columns) also show memory
size, disk speed, uptime and the 60s CPU peak per row.

**Stable order.** Rows keep their place between refreshes, so the row you look
at doesn't jump. `MOVE` shows how far a row would move if you sort again (`↑5`,
`↓7`), new processes get a `NEW` badge at the bottom, and `c` / `m` / `n` sorts
again.

**Heavy app alert.** A row that stays over 80% CPU for 30 seconds turns red
(`⚠`) and macOS shows a notification. It can alert again after it cools down.

**Multi-select.** Press `space` to mark several rows (`●`), then `k` to kill
them all at once — handy for clearing a swarm of leftovers. Protected rows are
skipped.

**Grouping.** By default, an app's helper processes are collapsed into one row
(`×9  Google Chrome`) showing combined CPU/RAM. Pressing `k` kills the whole
group. Press `t` to switch to a flat per-process view. Command-line tools
(node, python, …) stay as individual rows.

**Ports.** Processes listening on a TCP port show it inline (`node  :3000`).
From the command line, `overload --port 3000` lists what holds a port and
`overload --kill-port 3000` frees it — the cure for `EADDRINUSE`.

**Layout.** The top border of the process list shows the last action: `killing…`
after you press `k`, then `✓ killed` once the process is gone, staying visible
until your next action. It also shows the filter, marked and pinned counts.
Each row has a heat-colored CPU meter (green → yellow → red), and the selected
row stays near the center.

System-critical processes (WindowServer, kernel_task, launchd, securityd,
opendirectoryd, …) are shown greyed and cannot be killed — neither by SIGTERM
nor by the escalated SIGKILL. PIDs below 100 are protected too, and a recycled
PID is re-checked before the delayed SIGKILL so a reused slot can't be hit by
mistake.

## Requirements

- macOS (uses BSD signals and `/usr/sbin/lsof`).
- Port discovery shells out to `lsof`; it runs with a hard timeout so an
  unresponsive `lsof` (stale NFS/FUSE mount, slow resolution) can never freeze
  the UI — ports simply go unshown for that tick.

## Install

### Homebrew

```
brew install latreon/tap/overload
```

(or install the formula directly:
`brew install https://raw.githubusercontent.com/latreon/overload/main/Formula/overload.rb`)

### From source

```
cargo install --path .
# or:
cargo build --release && mv target/release/overload /usr/local/bin/
```
