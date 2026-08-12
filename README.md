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
- Cross-machine sync.

## Known limitations

- On Windows, `cwd`/`git_root` are lowercased before storage and lookup so
  that differently-cased spellings of the same directory (e.g. `C:\Dev\Foo`
  vs `c:\dev\foo`) match. This only applies going forward — rows written by
  a build predating this normalization keep their original casing.
- The lowest-weight "run anywhere" suggestion tier draws from a bounded,
  most-recent sample (500 rows) of history outside the current cwd/repo,
  not the full table, to keep `suggest` cheap on large histories.
- The ignore list (see "Ignoring commands") is a manual opt-out, not
  automatic secret detection — nothing scans command text for
  credential-shaped strings, so an un-listed command with a secret in it
  is still logged verbatim.
