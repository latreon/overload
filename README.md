# overload

A macOS terminal tool to find and kill resource hogs fast.

## Usage

```
overload
```

- `↑` / `↓` — move selection
- `k` — kill selected process (SIGTERM, then SIGKILL after 2s if it survives)
- `q` — quit

System-critical processes (WindowServer, kernel_task, launchd, …) are shown
greyed and cannot be killed.

## Install

```
cargo build --release
mv target/release/overload /usr/local/bin/
```
