use crate::rank::HistoryEntry;
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

/// Bound on the "exact cwd" / "same git repo" tiers (see
/// `rank::scope_weight`): the highest-weight matches, so we can afford a
/// generous cap.
const SCOPED_CANDIDATE_LIMIT: i64 = 5000;
/// Bound on the "everywhere else" fallback tier: lowest weight (0.15) in
/// `rank::scope_weight`, so a modest, most-recent-first sample is enough —
/// no need to scan the whole table for it.
const GLOBAL_CANDIDATE_LIMIT: i64 = 500;

pub struct Db {
    conn: Connection,
}

/// One history row as written by `cmdtrail export` — JSON Lines, one
/// object per line, ordered chronologically. Deliberately mirrors the
/// `commands` table columns directly rather than `rank::HistoryEntry`
/// (which omits `shell` since ranking doesn't use it); export is meant
/// to be a faithful dump, not a ranking input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExportRecord {
    pub command: String,
    pub cwd: String,
    pub git_root: Option<String>,
    pub shell: String,
    pub exit_code: Option<i32>,
    pub ts: i64,
}

/// Result of `Db::import`.
#[derive(Debug, PartialEq)]
pub struct ImportStats {
    pub inserted: usize,
    pub skipped: usize,
}

/// The cmdtrail data directory: `~/.local/share/cmdtrail/` on Linux/macOS,
/// `%APPDATA%\cmdtrail\` on Windows. Also holds `ignore.txt` (see
/// `crate::ignore`).
pub fn default_dir() -> Result<PathBuf> {
    let base = dirs::data_dir().context("could not resolve platform data directory")?;
    let dir = base.join("cmdtrail");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn default_db_path() -> Result<PathBuf> {
    Ok(default_dir()?.join("history.db"))
}

impl Db {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        // Default busy_timeout is 0: a second shell writing at the same
        // instant gets SQLITE_BUSY immediately instead of waiting, and the
        // shell hooks swallow that error silently (stderr redirected to
        // /dev/null) — so history entries vanish with no diagnostic under
        // ordinary multi-pane use. Give concurrent writers room to retry.
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS commands (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                command TEXT NOT NULL,
                cwd TEXT NOT NULL,
                git_root TEXT,
                shell TEXT NOT NULL,
                exit_code INTEGER,
                ts INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_commands_cwd ON commands(cwd);
            CREATE INDEX IF NOT EXISTS idx_commands_git_root ON commands(git_root);
            CREATE INDEX IF NOT EXISTS idx_commands_ts ON commands(ts);
            ",
        )?;
        Ok(Db { conn })
    }

    pub fn log(
        &self,
        command: &str,
        cwd: &str,
        git_root: Option<&str>,
        shell: &str,
        exit_code: Option<i32>,
        ts: i64,
    ) -> Result<()> {
        // Skip empty/whitespace-only lines and don't log the suggest/log
        // invocations themselves back into history.
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO commands (command, cwd, git_root, shell, exit_code, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![trimmed, cwd, git_root, shell, exit_code, ts],
        )?;
        Ok(())
    }

    /// Load candidate history entries for a target scope. Two tiers:
    /// - scoped: everything at the exact cwd, or (if in a git repo)
    ///   anywhere in the same repo — the `1.0`/`0.5` weights in
    ///   `rank::scope_weight`.
    /// - global: a bounded, most-recent sample of everything else — the
    ///   `0.15` fallback weight. Without this, directories/repos you've
    ///   never worked in before would get zero suggestions instead of a
    ///   low-weight "commands you run everywhere" fallback.
    ///
    /// Ranking/filtering happens in `rank::rank`, not here — this is just a
    /// bounded fetch so we're not loading the whole table on every call.
    pub fn candidates(&self, target_cwd: &str, target_git_root: Option<&str>) -> Result<Vec<HistoryEntry>> {
        let mut out = Vec::new();

        if let Some(root) = target_git_root {
            let mut scoped = self.conn.prepare(
                "SELECT command, cwd, git_root, exit_code, ts FROM commands
                 WHERE cwd = ?1 OR git_root = ?2
                 ORDER BY ts DESC LIMIT ?3",
            )?;
            for r in scoped.query_map(params![target_cwd, root, SCOPED_CANDIDATE_LIMIT], row_to_entry)? {
                out.push(r?);
            }

            let mut global = self.conn.prepare(
                "SELECT command, cwd, git_root, exit_code, ts FROM commands
                 WHERE cwd != ?1 AND (git_root IS NULL OR git_root != ?2)
                 ORDER BY ts DESC LIMIT ?3",
            )?;
            for r in global.query_map(params![target_cwd, root, GLOBAL_CANDIDATE_LIMIT], row_to_entry)? {
                out.push(r?);
            }
        } else {
            let mut scoped = self.conn.prepare(
                "SELECT command, cwd, git_root, exit_code, ts FROM commands
                 WHERE cwd = ?1
                 ORDER BY ts DESC LIMIT ?2",
            )?;
            for r in scoped.query_map(params![target_cwd, SCOPED_CANDIDATE_LIMIT], row_to_entry)? {
                out.push(r?);
            }

            let mut global = self.conn.prepare(
                "SELECT command, cwd, git_root, exit_code, ts FROM commands
                 WHERE cwd != ?1
                 ORDER BY ts DESC LIMIT ?2",
            )?;
            for r in global.query_map(params![target_cwd, GLOBAL_CANDIDATE_LIMIT], row_to_entry)? {
                out.push(r?);
            }
        }

        Ok(out)
    }

    /// Write every history row as JSON Lines (one JSON object per line),
    /// oldest first, to `out`. Streams row-by-row rather than collecting
    /// into a `Vec` first, so this stays cheap on very large histories.
    /// Returns the number of rows written.
    pub fn export_all<W: Write>(&self, out: &mut W) -> Result<usize> {
        let mut stmt = self.conn.prepare(
            "SELECT command, cwd, git_root, shell, exit_code, ts FROM commands ORDER BY ts ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ExportRecord {
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
            let record = r?;
            serde_json::to_writer(&mut *out, &record).context("failed to serialize history record")?;
            out.write_all(b"\n")?;
            count += 1;
        }
        Ok(count)
    }

    /// Count rows with `ts < cutoff_ts`, without deleting anything — the
    /// `cmdtrail prune --dry-run` query.
    pub fn count_older_than(&self, cutoff_ts: i64) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM commands WHERE ts < ?1",
            params![cutoff_ts],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Delete rows with `ts < cutoff_ts`. Returns the number of rows
    /// deleted. Does not reclaim disk space on its own — see `vacuum`.
    pub fn prune_older_than(&self, cutoff_ts: i64) -> Result<usize> {
        let deleted = self.conn.execute("DELETE FROM commands WHERE ts < ?1", params![cutoff_ts])?;
        Ok(deleted)
    }

    /// Rewrite the whole database file to reclaim space freed by deletes.
    /// Opt-in and separate from `prune_older_than`: `VACUUM` rewrites the
    /// entire file, which is comparatively expensive and briefly needs
    /// up to ~2x the DB's disk space, so it shouldn't happen implicitly
    /// on every prune.
    pub fn vacuum(&self) -> Result<()> {
        self.conn.execute_batch("VACUUM")?;
        Ok(())
    }

    /// Merge `records` into the table, skipping any that already exist
    /// (exact match on every column, including `ts`) so re-importing the
    /// same export file — or overlapping exports from multiple machines —
    /// is idempotent. This is the whole of "cross-machine sync": point
    /// your own file-sync tool (Dropbox/Syncthing/git/rsync/...) at each
    /// machine's `cmdtrail export` output, then `cmdtrail import` it
    /// wherever it shows up. No network code, no live-database sharing —
    /// syncing a SQLite+WAL file directly across machines via a generic
    /// file-sync tool risks corruption, so this never does that.
    ///
    /// Requires `&mut self` (unlike every other `Db` method) because
    /// rusqlite's transaction API needs an exclusive borrow; wrapping the
    /// whole import in one transaction keeps a large import atomic and
    /// fast (autocommit-per-row would be both slower and leave a partial
    /// import on failure).
    pub fn import<I: IntoIterator<Item = ExportRecord>>(&mut self, records: I) -> Result<ImportStats> {
        let tx = self.conn.transaction()?;
        let mut inserted = 0usize;
        let mut skipped = 0usize;
        {
            let mut exists_stmt = tx.prepare(
                "SELECT 1 FROM commands
                 WHERE command = ?1 AND cwd = ?2 AND git_root IS ?3
                   AND shell = ?4 AND exit_code IS ?5 AND ts = ?6
                 LIMIT 1",
            )?;
            let mut insert_stmt = tx.prepare(
                "INSERT INTO commands (command, cwd, git_root, shell, exit_code, ts)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for r in records {
                let dup = exists_stmt
                    .exists(params![r.command, r.cwd, r.git_root, r.shell, r.exit_code, r.ts])?;
                if dup {
                    skipped += 1;
                    continue;
                }
                insert_stmt.execute(params![r.command, r.cwd, r.git_root, r.shell, r.exit_code, r.ts])?;
                inserted += 1;
            }
        }
        tx.commit()?;
        Ok(ImportStats { inserted, skipped })
    }
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        command: row.get(0)?,
        cwd: row.get(1)?,
        git_root: row.get(2)?,
        exit_code: row.get(3)?,
        ts: row.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique per-test scratch DB path; not a real sqlite extension
    /// requirement, just avoids collisions between concurrent test runs.
    fn temp_db_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "cmdtrail-dbtest-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn cleanup(path: &std::path::Path) {
        std::fs::remove_file(path).ok();
        std::fs::remove_file(format!("{}-wal", path.display())).ok();
        std::fs::remove_file(format!("{}-shm", path.display())).ok();
    }

    /// Regression test for the "everywhere else" tier being unreachable
    /// through the real `Db` -> `rank::rank` path (it used to only show up
    /// in rank.rs's own unit tests, which hand-build entries and never
    /// exercise the SQL scoping here).
    #[test]
    fn candidates_reach_all_three_scope_tiers() {
        let path = temp_db_path("tiers");
        let db = Db::open(&path).unwrap();

        db.log("cmd_exact", "/repo/this-crate", Some("/repo"), "bash", Some(0), 100).unwrap();
        db.log("cmd_repo", "/repo/other-crate", Some("/repo"), "bash", Some(0), 100).unwrap();
        db.log("cmd_global", "/somewhere/else", None, "bash", Some(0), 100).unwrap();

        let entries = db.candidates("/repo/this-crate", Some("/repo")).unwrap();
        let commands: Vec<&str> = entries.iter().map(|e| e.command.as_str()).collect();

        assert!(commands.contains(&"cmd_exact"));
        assert!(commands.contains(&"cmd_repo"));
        assert!(
            commands.contains(&"cmd_global"),
            "global fallback tier must be reachable through Db::candidates, not just rank::rank"
        );

        cleanup(&path);
    }

    #[test]
    fn candidates_outside_any_repo_still_reaches_global_tier() {
        let path = temp_db_path("no-repo");
        let db = Db::open(&path).unwrap();

        db.log("cmd_exact", "/scratch", None, "bash", Some(0), 100).unwrap();
        db.log("cmd_elsewhere", "/repo/crate", Some("/repo"), "bash", Some(0), 100).unwrap();

        let entries = db.candidates("/scratch", None).unwrap();
        let commands: Vec<&str> = entries.iter().map(|e| e.command.as_str()).collect();

        assert!(commands.contains(&"cmd_exact"));
        assert!(
            commands.contains(&"cmd_elsewhere"),
            "a target outside any git repo must still see the global fallback tier"
        );

        cleanup(&path);
    }

    #[test]
    fn export_all_writes_chronological_jsonl() {
        let path = temp_db_path("export");
        let db = Db::open(&path).unwrap();

        db.log("second", "/repo", None, "bash", Some(0), 200).unwrap();
        db.log("first", "/repo", None, "bash", Some(1), 100).unwrap();

        let mut buf: Vec<u8> = Vec::new();
        let count = db.export_all(&mut buf).unwrap();
        assert_eq!(count, 2);

        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(first["command"], "first");
        assert_eq!(first["exit_code"], 1);
        assert_eq!(second["command"], "second");

        cleanup(&path);
    }

    #[test]
    fn prune_deletes_only_entries_older_than_cutoff() {
        let path = temp_db_path("prune");
        let db = Db::open(&path).unwrap();

        db.log("old", "/repo", None, "bash", Some(0), 100).unwrap();
        db.log("new", "/repo", None, "bash", Some(0), 1000).unwrap();

        assert_eq!(db.count_older_than(500).unwrap(), 1);

        let deleted = db.prune_older_than(500).unwrap();
        assert_eq!(deleted, 1);

        let remaining = db.candidates("/repo", None).unwrap();
        let commands: Vec<&str> = remaining.iter().map(|e| e.command.as_str()).collect();
        assert_eq!(commands, vec!["new"]);

        cleanup(&path);
    }

    #[test]
    fn prune_dry_run_via_count_does_not_delete() {
        let path = temp_db_path("prune-dry-run");
        let db = Db::open(&path).unwrap();

        db.log("old", "/repo", None, "bash", Some(0), 100).unwrap();

        // Simulate --dry-run: only count, never call prune_older_than.
        assert_eq!(db.count_older_than(500).unwrap(), 1);

        let remaining = db.candidates("/repo", None).unwrap();
        assert_eq!(remaining.len(), 1, "dry-run must not have deleted anything");

        cleanup(&path);
    }

    fn record(cmd: &str, cwd: &str, ts: i64) -> ExportRecord {
        ExportRecord {
            command: cmd.to_string(),
            cwd: cwd.to_string(),
            git_root: None,
            shell: "bash".to_string(),
            exit_code: Some(0),
            ts,
        }
    }

    #[test]
    fn import_inserts_new_entries() {
        let path = temp_db_path("import-new");
        let mut db = Db::open(&path).unwrap();

        let stats = db
            .import(vec![record("a", "/repo", 100), record("b", "/repo", 200)])
            .unwrap();
        assert_eq!(stats, ImportStats { inserted: 2, skipped: 0 });

        let entries = db.candidates("/repo", None).unwrap();
        assert_eq!(entries.len(), 2);

        cleanup(&path);
    }

    #[test]
    fn reimporting_the_same_records_is_fully_deduped() {
        let path = temp_db_path("import-dedup");
        let mut db = Db::open(&path).unwrap();

        let records = vec![record("a", "/repo", 100), record("b", "/repo", 200)];
        db.import(records.clone()).unwrap();
        let second = db.import(records).unwrap();

        assert_eq!(second, ImportStats { inserted: 0, skipped: 2 });
        assert_eq!(db.candidates("/repo", None).unwrap().len(), 2);

        cleanup(&path);
    }

    #[test]
    fn import_reports_partial_overlap_correctly() {
        let path = temp_db_path("import-partial");
        let mut db = Db::open(&path).unwrap();

        db.import(vec![record("a", "/repo", 100)]).unwrap();
        let stats = db
            .import(vec![record("a", "/repo", 100), record("c", "/repo", 300)])
            .unwrap();

        assert_eq!(stats, ImportStats { inserted: 1, skipped: 1 });
        assert_eq!(db.candidates("/repo", None).unwrap().len(), 2);

        cleanup(&path);
    }

    #[test]
    fn import_treats_differing_ts_as_a_distinct_entry() {
        let path = temp_db_path("import-distinct-ts");
        let mut db = Db::open(&path).unwrap();

        db.import(vec![record("a", "/repo", 100)]).unwrap();
        let stats = db.import(vec![record("a", "/repo", 999)]).unwrap();

        assert_eq!(stats, ImportStats { inserted: 1, skipped: 0 });
        assert_eq!(db.candidates("/repo", None).unwrap().len(), 2);

        cleanup(&path);
    }
}
