# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`overload` — a macOS-only Rust TUI (ratatui + crossterm) that lists processes by CPU/RAM and kills them
with one keypress. Single binary, no config files, no async runtime. ~2400 lines across 5 modules,
all tests live in `#[cfg(test)] mod tests` at the bottom of each source file.

## Commands

```bash
cargo build                      # debug build
cargo build --release            # release binary → target/release/overload
cargo test                       # all tests
cargo test killer::               # one module's tests
cargo test killer::tests::kill_refuses_protected -- --exact  # single test
cargo clippy --all-targets       # lint
cargo fmt                        # format
cargo run -- --port 3000         # CLI mode (no TUI, safe to run non-interactively)
```

Do **not** run `cargo run` with no args from an agent session — it takes over the terminal with the
alternate screen. Exercise the TUI through `render()`/`App::handle_key` tests with `TestBackend` instead
(see `src/main.rs` tests).

Several tests spawn real `sleep` processes and send them real signals (`killer.rs`), and
`collector.rs` tests read the live process table with a 300 ms sleep to prime CPU deltas. They are
slow-ish and machine-dependent by design; don't mock them away.

## Architecture

Data flows one way each 1500 ms tick, driven by the loop in `main.rs::run`:

```
collector::snapshot()  → Vec<ProcRow>  (pid, name, cpu%, ram%, memory, disk/s, run time, state)
        ↓ annotated with ports::ports_by_pid()
ui::App::set_rows()    → filter → group → sort → keep order → pins → Vec<DisplayRow>
        ↓ then history samples and heavy-app alerts (new_alerts → main::notify)
        ↓
main::render()         → ratatui frame
        ↑ keypresses
ui::App::handle_key()  → Action::{None, Quit, Kill(pids, label)}
        ↓
killer::kill()         → SIGTERM now, SIGKILL after 2s on a detached thread
```

Module responsibilities:

- **`collector`** — the only place `sysinfo` is touched. Owns the `System` and `total_mem`, and
  times snapshots (`last_snapshot`) to turn disk byte counts into per-second rates.
  sysinfo reports 0% CPU on a first read, so `run_tui` primes with a throwaway snapshot + 250 ms
  sleep before the first real one; keep that or the first frame sorts wrong.
- **`classifier`** — pure `classify(name, pid) -> Protection`. The single source of truth for
  "may not be killed": a hardcoded name list plus `pid < 100`. No I/O, trivially testable.
- **`killer`** — signals via `nix`. `kill()` re-checks protection before SIGTERM, and the deferred
  SIGKILL thread calls `should_sigkill()` to re-classify: a PID can be recycled during the 2 s
  window, so the escalation must never fire blind. Preserve that re-check.
- **`ports`** — shells out to a hardcoded `/usr/sbin/lsof` (not `PATH`, since its output feeds
  process killing) behind a 1.5 s `recv_timeout`. A hung `lsof` must degrade to "no ports shown",
  never block the UI. `parse_lsof` handles the `-F pcn` field-per-line format and dedupes (pid, port).
- **`ui`** — all state and the key reducer, no rendering. `App::rebuild()` recomputes `display` from
  `raw` and preserves the cursor by matching `DisplayRow::key` across refreshes. In grouped mode the
  key is the app group name (`app_group()` trims ` Helper` / ` (…)` suffixes); ungrouped it's
  `pid:<n>`. Marks (`marked`), pins (`pinned`), per-row CPU history (`history`) and alerts are keyed
  the same way, so they survive refreshes but reset when you toggle grouping.
  `rebuild(keep_order)`: refreshes and filtering pass `true`, so rows keep their previous position
  and new rows go to the bottom; only `c`/`m`/`n`/`t` pass `false` for a fresh sort, which also
  resets `sorted_keys` (rows not in it get the `NEW` badge) and `sorted_at`. `DisplayRow::moves` is
  how far a row would move on a fresh sort (positive = up). Pinned rows always come first.
  `check_alerts()` fires once per row that stays at `ALERT_CPU` for `ALERT_SAMPLES` refreshes.
  Confirmed kills go to `kill_log` (shown in the `?` help popup).
- **`main`** — arg parsing (`parse_args` → `Cmd`), all rendering helpers (`bar`, `spark`, `gradient`,
  `tile`, `format_bytes`, `format_duration`, `why_slow`), the help popup and the event loop.
  The top is four stat tiles plus one info line; the last action lives in the table's top border.
  Table columns depend on width (`WIDE_TABLE`, `EXTRA_WIDE_TABLE`); on wide screens TREND gets the
  space left after the fixed columns. `notify()` runs `/usr/bin/osascript` with the message as an
  argument, never inside the script text. `TerminalGuard` restores raw mode + the alternate screen
  on drop, including on panic.

## Invariants worth not breaking

- Kill is instant — no confirmation prompt. That's the product.
- Protection is enforced in three places on purpose: `ui` (won't emit the `Kill` action),
  `killer::kill` (rejects before SIGTERM), and `should_sigkill` (rejects before escalation).
- `render()` never blocks and never does I/O; all data arrives via `App` and `SysStats`.
- Rows keep their order between refreshes; only an explicit sort key re-sorts. The selected row also
  stays near the vertical center (offset math in `render`) so the cursor doesn't jump.

## Release

Version lives in `Cargo.toml` and must be mirrored in `Formula/overload.rb` (`url` tag + `sha256`
of the GitHub tarball). Tag `v<version>`, then bump the formula.
