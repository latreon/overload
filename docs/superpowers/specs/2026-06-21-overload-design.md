# overload — design

**Date:** 2026-06-21
**Status:** Approved, ready for implementation plan

## Problem

When the Mac heats up and the fan spins, finding the culprit is slow. Activity
Monitor hides orphaned processes (e.g. leftover automation browsers like
"Google Chrome for Testing"), shows confusing helper-process sprawl, and takes
several clicks to kill anything. The user hits this repeatedly and wants a
fast, no-friction way to see the top resource hogs and kill them.

## Goal

A terminal tool, `overload`, that shows a live list of processes sorted by CPU,
with one keypress to kill any process. Optimized for "what's cooking my Mac →
gone" in a couple of seconds.

## Non-goals (v1 / YAGNI)

- Per-app grouping of helper processes
- "Suspicious / orphan" auto-flagging
- RAM-sort toggle
- Config file
- GPU / thermal / power stats
- Cross-platform support (macOS only for v1; `sysinfo` keeps the door open)

## Tech

- **Language:** Rust
- **TUI:** Ratatui (+ crossterm for input/terminal)
- **Process data:** `sysinfo` crate (CPU%, RAM%, PID, name, parent PID — no
  fragile `ps`/`top` text parsing)
- **Distribution:** `cargo build --release` → single static binary dropped in
  `/usr/local/bin/overload`. No runtime, no venv.

## Architecture

Four small, independently-testable units communicating through plain function
calls.

### 1. Collector
Wraps `sysinfo`. Every ~1.5s produces a snapshot: a `Vec` of process rows
`{ pid, name, cpu_pct, ram_pct, parent_pid }`, sorted by CPU descending.
Single responsibility: produce a sorted snapshot.

### 2. Classifier
Pure function `classify(row) -> Killable | Protected`. Marks a process
`Protected` if it matches a system denylist. No side effects → fully
unit-testable.

Protected set (initial):
- Names: `WindowServer`, `kernel_task`, `launchd`, `loginwindow`,
  `WindowManager`, `coreaudiod`, `mds`, `mds_stores`
- Any process with PID < 100
- Processes owned by root that belong to the system (heuristic; refine during
  implementation)

### 3. Killer
`kill(pid)`:
1. Refuse immediately if the process is `Protected` (defense-in-depth — the UI
   already blocks it).
2. Send `SIGTERM`.
3. Spawn a deferred check: after 2s, if the PID is still alive, send
   `SIGKILL`.

Graceful when possible, dead when not.

### 4. UI (Ratatui)
Renders the table, owns cursor selection + scroll state, handles keys. Calls
Collector for data and Killer on `k`.

## Screen

```
 overload                          load 5.2 ↓   42 procs
 PID    CPU    RAM   NAME
▸2344   130%   1.2%  Dia Helper (Renderer)        [k]
 8981    99%   2.0%  python main.py               [k]
 8360    82%   0.4%  Google Chrome for Testing    [k]
 -- WindowServer      42%   0.6%  · protected --
 ...
 [↑↓] move   [k] kill   [q] quit
```

- Full scrollable list, sorted by CPU descending, top hog highlighted.
- Protected rows rendered greyed; `k` is ignored on them.

### Keys
- `↑` / `↓` — move cursor
- `k` — kill selected (instant, no confirm prompt; protected rows ignored)
- `q` — quit

(`/` filter is a possible later addition, not in v1.)

## Kill flow

`k` on a killable row → SIGTERM → row shows `killing…` → 2s later if still
alive, SIGKILL → process disappears on next refresh.

## Error handling

- **Process died between snapshot and kill:** treat as success, ignore.
- **Permission denied** (another user's / root process): flash
  `permission denied` in the status bar; never crash.
- **`sysinfo` read fails:** keep last good snapshot, show a stale marker.

## Testing

- **Classifier:** unit tests for the protected denylist and edge PIDs.
- **Killer:** spawn a dummy `sleep` process — assert SIGTERM kills it, assert a
  protected PID is refused.
- **Collector:** snapshot-shape / sort-order test.
- **UI:** thin; minimal coverage (selection clamp, scroll bounds).

## Install

```
cargo build --release
mv target/release/overload /usr/local/bin/
overload
```
