<div align="center">

# overload

**Find and kill the apps that slow down your Mac, right from the terminal.**

[![Release](https://img.shields.io/github/v/release/latreon/overload?color=5fd7ff)](https://github.com/latreon/overload/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-7ee787)](LICENSE)
![macOS](https://img.shields.io/badge/platform-macOS-lightgrey)
![Rust](https://img.shields.io/badge/built%20with-Rust-orange)

![overload demo: stat tiles, stable list with MOVE arrows, pin and the help popup](docs/demo.gif)

</div>

## Install

```bash
brew tap latreon/tap     # once
brew install overload
```

Update later with `brew upgrade overload`.

> If Homebrew says the tap is untrusted, run `brew trust latreon/tap` once and
> install again.

## Quick start

```bash
overload                     # open the live list
overload --port 3000         # what is listening on port 3000
overload --kill-port 3000    # free port 3000 (fixes EADDRINUSE)
overload --help              # all options
```

Open it, look at the red rows, select one and press `k`. That's it.

## Keys

| Key | Action |
| --- | --- |
| `j` `↓` / `↑` | Move down / up |
| `g` / `G` | Jump to top / bottom (`PgUp` / `PgDn` page) |
| `/` | Filter by name (`Esc` clears, `Enter` keeps it) |
| `c` `m` `n` | Sort by CPU, memory or name |
| `t` | Group an app's helper processes on or off |
| `p` | Pin the row on top (press again to unpin) |
| `space` | Mark a row (`Esc` clears all marks) |
| `k` | Kill the row, or every marked row |
| `K` | Kill only the busiest process of the row |
| `?` | Help with all keys and your recent kills |
| `q` | Quit |

## Features

### See what is wrong at a glance

- **Four tiles** at the top: CPU (with recent history), memory, swap and the Mac
  (load for 1, 5 and 15 minutes, uptime, process count).
- **Calm colors.** Tile numbers turn yellow at 70% and red at 90%. In the list,
  CPU meters go from green to red as an app gets heavier.
- **"Why slow?" line** in plain words, for example
  `⚠ node uses 95% CPU · swap 1.2 GB in use`.
- **Per-row trend:** a small chart of each app's CPU over the last minute and a
  half. You see at once if it is climbing or calming down.

### A list that doesn't jump

- **Rows keep their place** between refreshes. The row you look at stays put
  until you press `c`, `m` or `n` to sort again.
- **`MOVE`** shows how far a row would move if you sort now (`↑5`, `↓7`).
- **`NEW`** marks apps that started after your last sort. They wait at the
  bottom.
- **Pin** an app with `p` to keep it on top while you watch it.

### More detail on wide screens

| Terminal width | Extra columns |
| --- | --- |
| under 120 | `MOVE` `PID` `NAME` `CPU` `RAM` `TREND` |
| 120 and up | adds `MEM` `READ/s` `WRITE/s` `UPTIME` `PEAK` |
| 140 and up | adds `STATE` (running, sleeping, …) and `AVG` |
| 160 and up | adds `FOLDER`, the folder a process was started from (`~/work/api`) |

`FOLDER` tells apart several `node` or `python` processes, such as dev servers. It is
shown for single-process rows. Grouped rows (`×20 Google Chrome`) leave it empty, and so
do processes macOS does not let you inspect. Long paths keep their end: `…/clients/api`.

### Heavy app alert

An app that stays over **80% CPU for 30 seconds** turns red (`⚠`) and macOS
shows a notification. It can alert again after it cools down.

> The first time, macOS may ask to allow notifications from **Script Editor**.
> overload sends the notice with the built-in `osascript`. Allow it to see
> alerts.

### Kill the right thing

- **Groups:** an app's helpers show as one row (`×20  Google Chrome`) with
  combined CPU and memory. `k` kills the whole app, `K` only its busiest
  process. Press `t` for one row per process.
- **Batch kill:** mark rows with `space` (`●`), then press `k` once.
- **Ports:** apps listening on a TCP port show it next to the name
  (`node  :3000`).
- **Clear feedback:** the list's top border shows `killing…`, then
  `✓ killed`. `?` lists your recent kills.

## Safety

- System processes (WindowServer, kernel_task, launchd, securityd,
  opendirectoryd, …) and every PID below 100 are grey and **can't be killed**.
- A kill sends `SIGTERM` first and `SIGKILL` only if the app is still alive
  after 2 seconds.
- Before that `SIGKILL`, overload checks the PID again. If macOS gave the
  number to a new protected process in the meantime, nothing is sent.
- Port lookup runs `/usr/sbin/lsof` with a hard timeout. If `lsof` hangs, ports
  are simply not shown for that refresh and the screen never freezes.

## Requirements

- macOS (uses BSD signals and `/usr/sbin/lsof`)
- A terminal at least 80 columns wide. 140+ shows every column.

## Build from source

```bash
cargo install --git https://github.com/latreon/overload
```

Or from a clone:

```bash
cargo install --path .
```

## License

[MIT](LICENSE)
