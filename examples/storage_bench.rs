//! Standalone micro-benchmark: SQLite (cmdtrail's real storage) vs a flat
//! JSON Lines file (the format Claude Code-style transcripts use, and the
//! same format `cmdtrail export` already writes).
//!
//! This is an isolated harness — it never touches a real cmdtrail
//! database. It uses `cmdtrail::db::Db` directly (the actual production
//! code path, not a reimplementation) so the SQLite numbers reflect what
//! `cmdtrail suggest` really does.
//!
//! Scenario, chosen to match a realistic worst case rather than a
//! favorable one: a directory you don't visit often (a fixed 20 rows)
//! inside a shell history that's otherwise huge (N total rows spread
//! across many other directories). This isolates the question an index
//! actually answers: "how expensive is finding a small subset inside a
//! large table," which is exactly what `cmdtrail suggest` does on every
//! invocation.
//!
//! Run: `cargo run --release --example storage_bench`

use cmdtrail::db;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

const SIZES: &[usize] = &[1_000, 10_000, 100_000];
/// Rows that belong to the one directory we run `suggest` against,
/// regardless of N — see module doc for why this is fixed, not scaled.
const TARGET_MATCHES: usize = 20;
/// How many distinct "noise" directories the remaining rows spread across.
const NOISE_DIRS: usize = 500;

const SAMPLE_COMMANDS: &[&str] = &[
    "git status",
    "git commit -am wip",
    "git push",
    "cargo build",
    "cargo test",
    "cargo clippy",
    "npm install",
    "npm run dev",
    "ls -la",
    "docker compose up",
    "kubectl get pods",
    "vim src/main.rs",
    "python manage.py runserver",
    "make",
    "ssh prod",
    "grep -r TODO .",
    "go build ./...",
    "terraform apply",
    "psql mydb",
    "curl localhost:3000",
];

fn synth_command(i: usize) -> &'static str {
    SAMPLE_COMMANDS[i % SAMPLE_COMMANDS.len()]
}

/// Synthesize row `i`'s (cwd, git_root): the first TARGET_MATCHES rows
/// land in the target directory, the rest spread across NOISE_DIRS other
/// directories with no git repo (worst case for the "global" tier: it
/// can't rely on a git_root index hit either).
fn synth_scope(i: usize, target_cwd: &str, target_git_root: &str) -> (String, Option<String>) {
    if i < TARGET_MATCHES {
        (target_cwd.to_string(), Some(target_git_root.to_string()))
    } else {
        (format!("/home/dev/other-{}", i % NOISE_DIRS), None)
    }
}

fn fmt(d: Duration) -> String {
    if d.as_secs_f64() >= 1.0 {
        format!("{:.3}s", d.as_secs_f64())
    } else {
        format!("{:.1}ms", d.as_secs_f64() * 1000.0)
    }
}

fn cleanup_sqlite(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

/// Full unindexed scan of the SQLite table, deserializing every row —
/// isolates "index vs no index" from "SQLite row decode vs JSON parse."
fn sqlite_full_scan(path: &Path) -> rusqlite::Result<usize> {
    let conn = rusqlite::Connection::open(path)?;
    let mut stmt = conn.prepare("SELECT command, cwd, git_root, shell, exit_code, ts FROM commands")?;
    let rows = stmt.query_map([], |row| {
        Ok(db::ExportRecord {
            command: row.get(0)?,
            cwd: row.get(1)?,
            git_root: row.get(2)?,
            shell: row.get(3)?,
            exit_code: row.get(4)?,
            ts: row.get(5)?,
        })
    })?;
    let mut count = 0usize;
    for r in rows {
        let _record = r?;
        count += 1;
    }
    Ok(count)
}

/// What a JSON-L-backed `suggest` would have to do: no index exists, so
/// finding "commands relevant to this directory" means reading and
/// parsing every line and filtering in application code.
fn jsonl_scan_and_filter(path: &Path, target_cwd: &str, target_git_root: &str) -> anyhow::Result<usize> {
    let content = std::fs::read_to_string(path)?;
    let mut matches = 0usize;
    for line in content.lines() {
        if line.is_empty() {
            continue;
        }
        let record: db::ExportRecord = serde_json::from_str(line)?;
        if record.cwd == target_cwd || record.git_root.as_deref() == Some(target_git_root) {
            matches += 1;
        }
    }
    Ok(matches)
}

fn run_size(n: usize) -> anyhow::Result<()> {
    let target_cwd = "/home/dev/target-project";
    let target_git_root = "/home/dev/target-project";

    println!("== N = {n} rows ({TARGET_MATCHES} in the target directory, rest spread across {NOISE_DIRS} others) ==");

    // ---------- SQLite: write ----------
    // One `Db::log` call per row, matching real usage: each shell prompt
    // fires exactly one `cmdtrail log` invocation as a separate process.
    let sqlite_path =
        std::env::temp_dir().join(format!("cmdtrail-bench-{n}-{}.sqlite", std::process::id()));
    cleanup_sqlite(&sqlite_path);
    let db_handle = db::Db::open(&sqlite_path)?;

    let t0 = Instant::now();
    for i in 0..n {
        let (cwd, git_root) = synth_scope(i, target_cwd, target_git_root);
        db_handle.log(synth_command(i), &cwd, git_root.as_deref(), "bash", Some(0), i as i64)?;
    }
    let sqlite_write = t0.elapsed();

    // ---------- SQLite: the real `suggest` query path ----------
    let t1 = Instant::now();
    let candidates = db_handle.candidates(target_cwd, Some(target_git_root))?;
    let sqlite_indexed_read = t1.elapsed();
    let sqlite_indexed_matches = candidates
        .iter()
        .filter(|c| c.cwd == target_cwd || c.git_root.as_deref() == Some(target_git_root))
        .count();

    // ---------- SQLite: full unindexed scan (isolates format vs index) ----------
    let t2 = Instant::now();
    let sqlite_scan_count = sqlite_full_scan(&sqlite_path)?;
    let sqlite_full_scan_time = t2.elapsed();

    cleanup_sqlite(&sqlite_path);

    // ---------- JSON-L: write ----------
    // One append + flush per row, matching the same "one process per
    // command" real-world write pattern (and how Claude Code-style
    // transcripts are written incrementally, not batched).
    let jsonl_path =
        std::env::temp_dir().join(format!("cmdtrail-bench-{n}-{}.jsonl", std::process::id()));
    let mut file = std::fs::File::create(&jsonl_path)?;

    let t3 = Instant::now();
    for i in 0..n {
        let (cwd, git_root) = synth_scope(i, target_cwd, target_git_root);
        let record = db::ExportRecord {
            command: synth_command(i).to_string(),
            cwd,
            git_root,
            shell: "bash".to_string(),
            exit_code: Some(0),
            ts: i as i64,
        };
        serde_json::to_writer(&mut file, &record)?;
        file.write_all(b"\n")?;
        file.flush()?;
    }
    let jsonl_write = t3.elapsed();
    drop(file);

    // ---------- JSON-L: the only possible read path (full scan) ----------
    let t4 = Instant::now();
    let jsonl_matches = jsonl_scan_and_filter(&jsonl_path, target_cwd, target_git_root)?;
    let jsonl_read = t4.elapsed();

    let _ = std::fs::remove_file(&jsonl_path);

    // Sanity: every backend must agree on how many rows are actually
    // relevant to the target directory, or the benchmark itself is wrong.
    assert_eq!(sqlite_indexed_matches, TARGET_MATCHES);
    assert_eq!(jsonl_matches, TARGET_MATCHES);
    assert_eq!(sqlite_scan_count, n);

    println!("  write:                 sqlite {:>9}   jsonl {:>9}", fmt(sqlite_write), fmt(jsonl_write));
    println!(
        "  suggest (real query):  sqlite {:>9}   [no index possible on a flat file]",
        fmt(sqlite_indexed_read)
    );
    println!("  full unindexed scan:   sqlite {:>9}   jsonl {:>9}", fmt(sqlite_full_scan_time), fmt(jsonl_read));
    println!();

    Ok(())
}

fn main() -> anyhow::Result<()> {
    println!("cmdtrail storage benchmark: SQLite (real Db::candidates) vs flat JSON Lines\n");
    for &n in SIZES {
        run_size(n)?;
    }
    Ok(())
}
