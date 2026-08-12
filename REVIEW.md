# cmdtrail — Code Review

Scope: `src/{main,rank,picker,git,db}.rs`, `hooks/*`, `Cargo.toml`, `README.md`.

Verified: `cargo build --release` (clean, no warnings), `cargo test --release`
(8/8 pass), `cargo clippy --release --all-targets` (2 cosmetic lints only).
No behavioral test failures — the issues below are logic/design gaps the
current test suite doesn't cover.

## High severity

### 1. Picker cleanup clears the wrong rows — corrupts the terminal on every use
`src/picker.rs:69-76`

```rust
let lines_drawn = (items.len().min(10) + 1) as u16;
queue!(out, cursor::MoveToColumn(0))?;
for _ in 0..lines_drawn {
    queue!(out, terminal::Clear(ClearType::CurrentLine), cursor::MoveUp(1))?;
}
```

Tracing `render()`'s cursor math: each call ends with `MoveUp(drawn + 1)`
(line 96), so after the loop the cursor sits back at the **start row S**
(same row `pick()` was invoked from), with the prompt on row S and the item
list on rows `S+1..S+drawn` **below** it. Cleanup then does
`Clear(CurrentLine)` + `MoveUp(1)` repeatedly — moving **upward**, into rows
that existed *before* the picker ran (prior shell/scrollback content), never
touching rows `S+1..S+drawn` where the item list was actually printed.

Net effect on every Ctrl+G:
- Rows above the invocation point get erased (content unrelated to the picker).
- The rendered item list below is left on screen as garbage.

Compounding bug: `lines_drawn` is computed from `items.len()` (the full
candidate list, up to 200 in `--pick` mode per `main.rs:84`), not the
`filtered`/`drawn` count from the *last* render. Any time the user types a
filter that narrows the visible list, the two numbers diverge further.

**Fix:** track `drawn` from the final `render()` call and, from row S, issue
a single `Clear(ClearType::FromCursorDown)` (mirrors what `render()` already
does before drawing) instead of walking upward.

### 2. The documented third ranking tier ("everywhere else") is unreachable in production
`src/db.rs:68-81` vs `src/rank.rs:33-45`, README "What it does" §3

`Db::candidates()` only ever fetches rows matching `cwd = ?1 OR git_root =
?2`. Consequence, by cases:

- `target_git_root` is `Some`: every returned row satisfies
  `cwd == target_cwd` OR `git_root == target_git_root` (the exact bound
  value) — so in `scope_weight`, whenever the `cwd` branch doesn't fire,
  `a == b` is *guaranteed* true. The `else 0.15` branch is dead code.
- `target_git_root` is `None`: SQL is `WHERE cwd = ?1` only — every row has
  `cwd == target_cwd`, so *only* the 1.0 branch ever fires. Zero
  cross-directory suggestions when you're outside a git repo.

So "commands run anywhere" (tier 3, weight 0.15) never surfaces through the
real CLI path — only through `rank::rank()`'s unit tests, which
hand-construct entries and bypass `Db::candidates()` entirely. There is no
integration test exercising `Db` + `rank` together, which is exactly the gap
that let this slip through.

**Fix:** either broaden the query (e.g. union in a bounded "recent commands
overall" fetch) to actually implement tier 3, or drop the claim from the
README/rank.rs doc comment and simplify `scope_weight` to two tiers.

## Medium severity

### 3. No `busy_timeout` — concurrent shells silently drop history
`src/db.rs:20-39`

rusqlite/SQLite default `busy_timeout` is 0. Two terminal tabs writing at the
same instant → `SQLITE_BUSY` returned immediately → `db.log()` returns `Err`
→ `main()` propagates it → the shell hooks discard stderr/exit code
(`>/dev/null 2>&1` in bash/zsh; the pwsh history handler's bare `& cmdtrail
log ...` isn't wrapped in the `try/catch` at all). Result: silent,
un-diagnosable history loss under ordinary multi-pane usage.

**Fix:** `conn.busy_timeout(Duration::from_millis(...))` (or `PRAGMA
busy_timeout`) in `Db::open`.

### 4. Terminal can be stranded in raw mode / hidden cursor on error
`src/picker.rs:17-79`

`enable_raw_mode()` + `cursor::Hide` happen up front; every `?` inside the
loop (`render()`, `event::read()`) returns early with no cleanup — no RAII
guard, no `Drop` impl, no catch-then-restore. A rare crossterm error (e.g.
stderr not a real console under some launchers) leaves the invoking shell's
terminal broken (no echo, raw input) until the user runs `reset`/`stty
sane`.

**Fix:** a guard struct that restores mode/cursor in `Drop`, or wrap the
loop body and restore before returning `Err`.

### 5. Case-sensitive path comparisons on Windows
`src/rank.rs:34`, `src/git.rs:6-16`

`cwd`/`git_root` are compared as raw strings with no canonicalization or
casefolding. `C:\Dev\Foo` and `c:\dev\foo` (same directory, different casing
across sessions/tools) are treated as distinct scopes — silently defeats the
"exact cwd" tier on the one platform this project's dev workstation targets.

**Fix:** normalize (canonicalize, or lowercase on Windows) before storing
and before comparing.

## Minor / nitpicks

- `main.rs:79` — `std::env::current_dir().unwrap()` panics instead of
  propagating via `?`/`anyhow`, inconsistent with the rest of the
  `Result`-based code.
- `Cargo.toml:12` — `clap` pinned to exact `=4.4.18` while `rusqlite`,
  `dirs`, `anyhow`, `crossterm` use loose ranges; no comment explaining the
  exact pin. Inconsistent dependency policy, will silently rot.
- `db.rs:73/79` — `LIMIT 5000 ORDER BY ts DESC` is a reasonable bound in
  isolation, but combined with the `OR git_root=?2` clause, a chatty
  shared-repo tier can crowd out an old-but-legitimate exact-cwd command
  before ranking ever sees it. Worth a doc note.
- No secret-redaction/ignore-list: full command text is stored verbatim,
  unencrypted, indefinitely (e.g. `curl -H "Authorization: Bearer …"`), with
  no opt-out. Tools in this space (atuin, etc.) typically offer an
  ignore-pattern. Worth documenting as a known limitation.
- Clippy (cosmetic only): `picker.rs:56-58` implicit saturating sub;
  `rank.rs:9-10` doc-list indentation.

## Positives

- `db.rs` uses parameterized queries throughout — no SQL injection surface.
- Shell hooks quote `"$cmd"`/`$line` correctly — no shell-injection surface
  from logged command text.
- `rank.rs` has solid, well-named unit tests covering each scoring tier,
  recency, frequency, failure discount, prefix filter, and dedup — genuinely
  defends the ranking contract (just doesn't reach the DB layer, per
  finding #2).
- `git.rs`'s `.git`-file-vs-dir handling correctly covers
  worktrees/submodules.
- WAL mode, indices on `cwd`/`git_root`, `trim()` + empty-skip on log —
  sensible baseline hygiene.
- Clean release build with LTO, no warnings.
