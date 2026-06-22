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
- `k` — kill the selected row, or all marked rows (SIGTERM, then SIGKILL after 2s)
- `q` — quit

**Live trends & gauges.** A gauge bar shows overall CPU and memory meters plus a
sparkline of recent total CPU. Every row carries a `TREND` sparkline of its own
recent CPU history, so you can see whether a process is climbing or settling —
not just its current number. Meters use a smooth green→red gradient.

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

**Layout.** A persistent action line sits at the very top: it shows `killing…`
after you press `k` and flips to `✓ killed` once the process is gone, staying
visible until your next action. Below it, a summary line shows load average,
global CPU%, memory used%, process count, and the current sort/group/filter
state. Each row has a heat-colored CPU/RAM meter (green → yellow → red), and the
selected row stays pinned near the center so it doesn't jump as the list
re-sorts. The list is always sorted highest-first by the active sort key.

System-critical processes (WindowServer, kernel_task, launchd, …) are shown
greyed and cannot be killed.

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
