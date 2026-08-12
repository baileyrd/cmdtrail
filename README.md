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

Each hook does two things: logs every command you run on your prompt, and
binds **Ctrl+G** to open an interactive type-to-filter picker (rendered by
the binary itself) scoped to your current directory. Selecting an entry
inserts it into your current line, ready to edit or run.

## CLI

    cmdtrail log <command> --cwd <dir> --shell <name> [--exit-code <n>]
    cmdtrail suggest [--cwd <dir>] [--limit N] [--query <prefix>] [--pick]
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

## Not yet built (phase 2 candidates)

- True ghost-text-as-you-type instead of a keybinding-triggered picker.
  Feasible per-shell but each needs a different mechanism: zsh via a custom
  `zsh-autosuggestions` strategy, PowerShell via a PSReadLine
  `ICommandPredictor` plugin (.NET, would need an IPC bridge to this
  binary), bash has no native equivalent without `ble.sh`.
- History pruning/export.
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
