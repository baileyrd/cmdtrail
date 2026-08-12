# cmdtrail

Per-directory command history capture and suggestion, for PowerShell, bash, and zsh.

## What it does

Every command you run gets logged with its working directory, git repo root
(if any), shell, exit code, and timestamp. `cmdtrail suggest` then ranks past
commands for wherever you currently are:

1. commands run in this exact directory rank highest
2. commands run elsewhere in the same git repo rank next
3. everything else ranks lowest

Within each tier, more recent and more frequent commands win, and commands
that previously failed (non-zero exit) are discounted but not hidden.

## Build

    cargo build --release
    # binary at target/release/cmdtrail — put it on PATH

## Install the shell hook

PowerShell (`$PROFILE`):

    cmdtrail init pwsh >> $PROFILE

bash (`~/.bashrc`):

    echo 'eval "$(cmdtrail init bash)"' >> ~/.bashrc

zsh (`~/.zshrc`):

    echo 'eval "$(cmdtrail init zsh)"' >> ~/.zshrc

Each hook does three things: logs every command you run on your prompt,
binds **Ctrl+G** to open an interactive type-to-filter picker (rendered by
the binary itself) scoped to your current directory, and (zsh only, see
below) shows an inline ghost-text suggestion as you type. Selecting a
picker entry, or accepting a ghost-text suggestion, inserts it into your
current line, ready to edit or run.

### Ghost-text-as-you-type (zsh only)

zsh shows the single best-ranked suggestion as dimmed text after your
cursor while you type, like zsh-autosuggestions — but backed by cmdtrail's
directory-aware ranking instead of a plain history search. Self-contained
(no zsh-autosuggestions plugin dependency): it's a `zle` `line-pre-redraw`
hook using zsh's own bundled `add-zle-hook-widget`. Press **Right arrow**
or **End** (with the cursor already at the end of the line) to accept.

It forks `cmdtrail suggest` synchronously on every edited keystroke
(debounced against unchanged buffers and mid-line cursor positions) — fine
on typical histories, possibly noticeable on very large ones. Set
`CMDTRAIL_GHOST_TEXT=0` before the hook loads to disable it and keep only
the Ctrl+G picker.

PowerShell and bash don't have this yet — see "Not yet built" for why.

## CLI

    cmdtrail log <command> --cwd <dir> --shell <name> [--exit-code <n>]
    cmdtrail suggest [--cwd <dir>] [--limit N] [--query <prefix>] [--pick]
    cmdtrail export [--out <path>]
    cmdtrail prune --older-than <duration> [--dry-run] [--vacuum]
    cmdtrail import <path>...
    cmdtrail init <bash|zsh|pwsh>

Data lives in a SQLite DB at your platform's data dir (e.g.
`~/.local/share/cmdtrail/history.db` on Linux, `%APPDATA%\cmdtrail\` on
Windows).

## Ignoring commands

Commands matching a pattern in `<data dir>/ignore.txt` are never logged —
skipped before the database write, not stored and then filtered. The file
is created (as a commented-out template) the first time `cmdtrail log` runs.
One pattern per line:

    # substring match, case-insensitive, matches anywhere in the command
    password

    # glob match against the whole command; '*' matches any run of chars
    curl*Authorization*
    export*API_KEY=*

Blank lines and lines starting with `#` are ignored.

## Export & pruning

`cmdtrail export` writes every history row as JSON Lines (one JSON object
per line, oldest first) to stdout, or to a file with `--out`:

    cmdtrail export --out history.jsonl

Each line: `{"command","cwd","git_root","shell","exit_code","ts"}`.

`cmdtrail prune --older-than <duration>` deletes entries older than a
given age. Duration is `<number><unit>` with unit `h`/`d`/`w`/`m`/`y`
(hours/days/weeks/~30-day months/~365-day years):

    cmdtrail prune --older-than 90d --dry-run   # report count only, no deletes
    cmdtrail prune --older-than 90d             # actually deletes
    cmdtrail prune --older-than 90d --vacuum    # also reclaims disk space

`--vacuum` rewrites the whole database file to reclaim space freed by the
delete; it's separate and opt-in because it's comparatively expensive and
briefly needs up to ~2x the DB's disk space.

## Cross-machine sync

`cmdtrail import <path>...` merges history from one or more
`cmdtrail export` JSON-L files into the local database, skipping any
entry that's an exact match (every column, including timestamp) of one
already present — so importing the same file twice, or two exports that
overlap, is idempotent.

That's the entire sync story: there's no network code and no sharing a
live database file between machines (syncing a SQLite+WAL file directly
through a generic file-sync tool risks corruption). Instead:

    # on each machine, periodically:
    cmdtrail export --out ~/Dropbox/cmdtrail/$(hostname).jsonl

    # point Dropbox/Syncthing/git/rsync/OneDrive/... at that folder, then
    # on each machine, periodically:
    cmdtrail import ~/Dropbox/cmdtrail/*.jsonl

Every machine converges to the union of everyone's history. Export size
only grows, so pair this with `cmdtrail prune` if that matters to you.

## Storage benchmark: SQLite vs JSON-L

`cargo run --release --example storage_bench` — an isolated harness
comparing cmdtrail's real SQLite storage (`db::Db`, the actual production
code, not a reimplementation) against a flat JSON Lines file (the format
`cmdtrail export` already writes, and what Claude Code-style transcripts
use). Scenario: a directory with a fixed 20 matching rows inside a shell
history that's otherwise huge (N rows spread across 500 other
directories) — the realistic case an index is supposed to help with.

Measured on this machine, before and after adding an index on `ts`
(`idx_commands_ts`) to fix the finding below:

| N       | write (sqlite / jsonl) | `suggest` real query (sqlite): before → after | full scan (sqlite / jsonl) |
|---------|-------------------------|--------------------------------------------------|------------------------------|
| 1,000   | 784ms / 204ms           | 1.4ms → 0.8ms                                     | 60.8ms / 1.8ms                |
| 10,000  | 9.5s / 2.9s             | 28.8ms → 0.9ms (32x)                              | 75.8ms / 63.0ms                |
| 100,000 | 110.9s / 25.3s          | 236.4ms → 0.8ms (295x)                            | 114.4ms / 77.1ms               |

Two findings, reported as measured rather than as expected:

- **Write**: SQLite is ~2-4x slower per row than a JSON-L append (one
  transaction + WAL commit vs one `write` + `flush`). In practice this
  doesn't matter: real `cmdtrail log` calls are separate process
  invocations from a shell hook, where OS process-spawn cost (single-digit
  to tens of ms) already dwarfs a sub-millisecond difference in commit
  cost.
- **Read — the one that matters, since zsh ghost-text calls `suggest` on
  every keystroke**: the *before* column above is the bug — at 100k rows,
  cmdtrail's real `suggest` query took 236ms, *slower* than a naive full
  linear scan of a flat JSON-L file (77ms). Root cause: the "everywhere
  else" fallback tier's `ORDER BY ts DESC LIMIT 500` had no index on
  `ts`, forcing a scan-and-sort of nearly the whole table even though the
  exact-cwd/same-repo tiers were properly indexed. Fixed: `CREATE INDEX
  idx_commands_ts ON commands(ts)` lets SQLite walk the index in `ts`
  order and stop at 500 matches instead of sorting everything. Confirmed
  by re-running this same benchmark — `suggest` is now flat at ~0.8-0.9ms
  regardless of N (295x faster at 100k rows), comfortably beating the
  JSON-L scan it used to lose to.

## Not yet built (phase 2 candidates)

- Ghost-text-as-you-type for PowerShell and bash (zsh has it — see above).
  - PowerShell needs a `PSReadLine` `ICommandPredictor` plugin, which
    Microsoft's own docs require as a *compiled* C# assembly (`dotnet
    build` against the PowerShell SDK NuGet package) — there's no
    documented PowerShell-script-only path.
  - bash has no native ghost-text hook at all. `ble.sh` is the only
    candidate, but its completion system (`complete -F func cmdname`) is
    per-command — it completes arguments of an already-typed command, not
    "replace the whole line with a different suggested command," which is
    the wrong shape for cmdtrail's suggestions.
  - Both are unimplemented by decision, not just unstarted: implementing
    either without a way to compile/run and verify it (no .NET SDK, no
    ble.sh install, no working WSL on the dev machine this was built on)
    would mean shipping untested shell-integration code.
- `command_stats` rollup table to fix candidate-window crowding and cut
  `suggest` read cost further. Design, deferred (not started):
  - Problem: `commands` is append-only with no dedup, so a command run
    thousands of times in one directory can fill the entire
    `SCOPED_CANDIDATE_LIMIT = 5000` window with just that one command,
    crowding out other distinct commands from that directory's candidate
    set entirely.
  - New table, one row per unique `(command, cwd)`: `command_stats
    (command, cwd, git_root, run_count, last_ts, last_exit_code, PRIMARY
    KEY (command, cwd))`. `commands` itself is untouched — still the
    full, append-only source of truth for `export`/`import`/`prune`.
  - Refresh is a full rebuild (`DELETE` + `INSERT ... GROUP BY` in one
    transaction), not incremental — simpler, no drift bugs. Triggered
    only during a detected idle gap on `cmdtrail log` (a command follows
    a >threshold gap since the previous one), never on rapid-fire
    typing, so the rebuild cost never lands on a latency-sensitive path.
    Also re-run right after `prune`/`import`, since both bulk-mutate
    `commands` and would otherwise leave `command_stats` stale until the
    next idle window.
  - Idle threshold: default should be sized off the *measured* rebuild
    cost at whatever history size is typical (the storage benchmark's
    "full scan" numbers are the closest proxy so far — tens to ~100ms at
    100k rows), with a comfortable safety margin, and must be
    user-configurable via an env var (same pattern as
    `CMDTRAIL_GHOST_TEXT`) — not a hardcoded constant.
  - The gap that makes this non-trivial: a rollup refreshed only on idle
    would miss commands run in the *current* active session, which are
    exactly the most valuable suggestions ("I ran this 5 minutes ago in
    this repo"). `suggest` must read `command_stats` for the bulk plus a
    small `WHERE ts > last_refresh_ts` query against raw `commands` for
    anything newer, merged at read time — not a straight swap of the
    fetch source.

## Known limitations

- On Windows, `cwd`/`git_root` are lowercased before storage and lookup so
  that differently-cased spellings of the same directory (e.g. `C:\Dev\Foo`
  vs `c:\dev\foo`) match. This only applies going forward — rows written by
  a build predating this normalization keep their original casing.
- The lowest-weight "run anywhere" suggestion tier draws from a bounded,
  most-recent sample (500 rows) of history outside the current cwd/repo,
  not the full table, to keep `suggest` cheap on large histories. Backed
  by an index on `ts` so the `ORDER BY ts DESC LIMIT 500` behind it stays
  cheap as history grows — see "Storage benchmark" for measured numbers.
- The ignore list (see "Ignoring commands") is a manual opt-out, not
  automatic secret detection — nothing scans command text for
  credential-shaped strings, so an un-listed command with a secret in it
  is still logged verbatim.
