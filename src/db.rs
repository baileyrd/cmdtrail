use crate::rank::HistoryEntry;
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::PathBuf;

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

    /// Load candidate history entries relevant to a target scope: everything
    /// under the same git repo (if any) plus everything at the exact cwd.
    /// Ranking/filtering happens in `rank::rank`, not here — this is just a
    /// bounded fetch so we're not loading the whole table on every call.
    pub fn candidates(&self, target_cwd: &str, target_git_root: Option<&str>) -> Result<Vec<HistoryEntry>> {
        let mut stmt = if target_git_root.is_some() {
            self.conn.prepare(
                "SELECT command, cwd, git_root, exit_code, ts FROM commands
                 WHERE cwd = ?1 OR git_root = ?2
                 ORDER BY ts DESC LIMIT 5000",
            )?
        } else {
            self.conn.prepare(
                "SELECT command, cwd, git_root, exit_code, ts FROM commands
                 WHERE cwd = ?1
                 ORDER BY ts DESC LIMIT 5000",
            )?
        };

        let rows = if let Some(root) = target_git_root {
            stmt.query_map(params![target_cwd, root], row_to_entry)?
        } else {
            stmt.query_map(params![target_cwd], row_to_entry)?
        };

        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
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
