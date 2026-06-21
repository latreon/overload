# overload

A macOS terminal tool to find and kill resource hogs fast.

## Usage

```
overload
```

- `↑` / `↓` — move selection
- `k` — kill selected process (SIGTERM, then SIGKILL after 2s if it survives)
- `q` — quit

The header line shows load average, global CPU%, memory used%, and process
count. Each row has a heat-colored CPU/RAM meter (green → yellow → red). The
selected row stays pinned near the center of the screen so it doesn't jump as
the list re-sorts. After a kill, the footer shows `killing…` and then
`✓ killed` once the process is gone.

System-critical processes (WindowServer, kernel_task, launchd, …) are shown
greyed and cannot be killed.

## Install

```
cargo build --release
mv target/release/overload /usr/local/bin/
```
