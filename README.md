# overload

A macOS terminal tool to find and kill resource hogs fast.

## Usage

```
overload
```

- `↑` / `↓` — move selection
- `k` — kill selected process (SIGTERM, then SIGKILL after 2s if it survives)
- `q` — quit

The process list is always sorted by CPU, highest first. A persistent action
line sits at the very top: it shows `killing…` after you press `k` and flips to
`✓ killed` once the process is gone, staying visible until your next action.
Below it, a summary line shows load average, global CPU%, memory used%, and
process count. Each row has a heat-colored CPU/RAM meter (green → yellow → red),
and the selected row stays pinned near the center so it doesn't jump as the
list re-sorts.

System-critical processes (WindowServer, kernel_task, launchd, …) are shown
greyed and cannot be killed.

## Install

```
cargo build --release
mv target/release/overload /usr/local/bin/
```
