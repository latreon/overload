# overload

A macOS terminal tool to find and kill resource hogs fast.

## Usage

```
overload
```

- `j` / `↑` `↓` — move selection
- `g` / `G` (or Home/End) — jump to top / bottom; `PgUp` / `PgDn` — page
- `/` — filter by name (type to narrow, `Esc` clears, `Enter` keeps it)
- `c` / `m` / `n` — sort by CPU / memory / name
- `t` — toggle grouping of an app's helper processes
- `k` — kill the selected row (SIGTERM, then SIGKILL after 2s if it survives)
- `q` — quit

**Grouping.** By default, an app's helper processes are collapsed into one row
(`×9  Google Chrome`) showing combined CPU/RAM. Pressing `k` kills the whole
group. Press `t` to switch to a flat per-process view. Command-line tools
(node, python, …) stay as individual rows.

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

```
cargo build --release
mv target/release/overload /usr/local/bin/
```
