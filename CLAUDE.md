# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`overload` — a macOS-only Rust TUI (ratatui + crossterm) that lists processes by CPU/RAM and kills them
with one keypress. Single binary, no config files, no async runtime. ~1800 lines across 5 modules,
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
collector::snapshot()  → Vec<ProcRow>  (pid, name, cpu%, ram%)
        ↓ annotated with ports::ports_by_pid()
ui::App::set_rows()    → filter → group → sort → Vec<DisplayRow>
        ↓
main::render()         → ratatui frame
        ↑ keypresses
ui::App::handle_key()  → Action::{None, Quit, Kill(pids, label)}
        ↓
killer::kill()         → SIGTERM now, SIGKILL after 2s on a detached thread
```

Module responsibilities:

- **`collector`** — the only place `sysinfo` is touched. Owns the `System` and `total_mem`.
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
  `pid:<n>`. Marks (`marked`) and per-row CPU history (`history`) are keyed the same way, so they
  survive refreshes but reset when you toggle grouping.
- **`main`** — arg parsing (`parse_args` → `Cmd`), all rendering helpers (`bar`, `spark`, `gradient`),
  and the event loop. `TerminalGuard` restores raw mode + the alternate screen on drop, including
  on panic.

## Invariants worth not breaking

- Kill is instant — no confirmation prompt. That's the product.
- Protection is enforced in three places on purpose: `ui` (won't emit the `Kill` action),
  `killer::kill` (rejects before SIGTERM), and `should_sigkill` (rejects before escalation).
- `render()` never blocks and never does I/O; all data arrives via `App` and `SysStats`.
- The selected row is pinned near the vertical center (offset math in `render`) so the cursor
  doesn't jump as the list re-sorts each tick.

## Release

Version lives in `Cargo.toml` and must be mirrored in `Formula/overload.rb` (`url` tag + `sha256`
of the GitHub tarball). Tag `v<version>`, then bump the formula.
