use crate::rank::HistoryEntry;
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
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

/// `~/.local/share/cmdtrail/history.db` on Linux/macOS,
/// `%APPDATA%\cmdtrail\history.db` on Windows.
pub fn default_db_path() -> Result<PathBuf> {
    let base = dirs::data_dir().context("could not resolve platform data directory")?;
    let dir = base.join("cmdtrail");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("history.db"))
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
}
