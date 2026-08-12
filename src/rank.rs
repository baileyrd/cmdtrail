//! Frecency-style ranking of past commands, scoped by directory.
//!
//! Scoring intentionally has three tiers so a command run only in this
//! exact directory always outranks one merely run somewhere else in the
//! same repo, which in turn outranks a command run anywhere:
//!   - exact cwd match:      weight 1.00
//!   - same git repo, other subdir: weight 0.50
//!   - everywhere else:      weight 0.15
//! Within a tier, more recent and more frequent runs score higher, and
//! runs that exited non-zero are discounted (still shown, just lower).

use std::collections::HashMap;

const DECAY_SECONDS: f64 = 14.0 * 24.0 * 3600.0; // half-life-ish window: ~2 weeks
const FAILED_EXIT_PENALTY: f64 = 0.5;

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub command: String,
    pub cwd: String,
    pub git_root: Option<String>,
    pub exit_code: Option<i32>,
    pub ts: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RankedCommand {
    pub command: String,
    pub score: f64,
    pub last_used: i64,
}

fn scope_weight(entry: &HistoryEntry, target_cwd: &str, target_git_root: Option<&str>) -> f64 {
    if entry.cwd == target_cwd {
        1.0
    } else if let (Some(a), Some(b)) = (entry.git_root.as_deref(), target_git_root) {
        if a == b {
            0.5
        } else {
            0.15
        }
    } else {
        0.15
    }
}

fn recency_weight(age_secs: f64) -> f64 {
    (-age_secs / DECAY_SECONDS).exp()
}

/// Rank `entries` for suggestion in `target_cwd` (optionally scoped further
/// by `target_git_root`), as of `now` (unix seconds). If `query_prefix` is
/// given, only commands whose text starts with it (case-insensitive) are
/// considered. Returns commands sorted best-first, deduplicated by exact
/// command text.
pub fn rank(
    entries: &[HistoryEntry],
    target_cwd: &str,
    target_git_root: Option<&str>,
    now: i64,
    query_prefix: Option<&str>,
    limit: usize,
) -> Vec<RankedCommand> {
    let prefix_lower = query_prefix.map(|p| p.to_lowercase());

    let mut scores: HashMap<String, (f64, i64)> = HashMap::new();

    for entry in entries {
        if let Some(p) = &prefix_lower {
            if !entry.command.to_lowercase().starts_with(p.as_str()) {
                continue;
            }
        }

        let age = (now - entry.ts).max(0) as f64;
        let mut s = scope_weight(entry, target_cwd, target_git_root) * (1.0 + recency_weight(age));

        if let Some(code) = entry.exit_code {
            if code != 0 {
                s *= FAILED_EXIT_PENALTY;
            }
        }

        let e = scores.entry(entry.command.clone()).or_insert((0.0, entry.ts));
        e.0 += s;
        if entry.ts > e.1 {
            e.1 = entry.ts;
        }
    }

    let mut ranked: Vec<RankedCommand> = scores
        .into_iter()
        .map(|(command, (score, last_used))| RankedCommand {
            command,
            score,
            last_used,
        })
        .collect();

    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap()
            .then_with(|| b.last_used.cmp(&a.last_used))
    });
    ranked.truncate(limit);
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(cmd: &str, cwd: &str, git_root: Option<&str>, exit: Option<i32>, ts: i64) -> HistoryEntry {
        HistoryEntry {
            command: cmd.to_string(),
            cwd: cwd.to_string(),
            git_root: git_root.map(|s| s.to_string()),
            exit_code: exit,
            ts,
        }
    }

    #[test]
    fn exact_cwd_outranks_repo_and_global() {
        let now = 1_000_000;
        let entries = vec![
            entry("cargo build", "/repo/other-crate", Some("/repo"), Some(0), now - 60),
            entry("cargo test", "/repo/this-crate", Some("/repo"), Some(0), now - 60),
            entry("ls -la", "/somewhere/else", None, Some(0), now - 60),
        ];
        let ranked = rank(&entries, "/repo/this-crate", Some("/repo"), now, None, 10);
        assert_eq!(ranked[0].command, "cargo test"); // exact cwd
        assert_eq!(ranked[1].command, "cargo build"); // same repo
        assert_eq!(ranked[2].command, "ls -la"); // global
    }

    #[test]
    fn frequency_increases_score() {
        let now = 1_000_000;
        let entries = vec![
            entry("git status", "/repo", Some("/repo"), Some(0), now - 100),
            entry("git status", "/repo", Some("/repo"), Some(0), now - 200),
            entry("git log", "/repo", Some("/repo"), Some(0), now - 50),
        ];
        let ranked = rank(&entries, "/repo", Some("/repo"), now, None, 10);
        assert_eq!(ranked[0].command, "git status");
    }

    #[test]
    fn recent_beats_old_at_equal_frequency() {
        let now = 1_000_000;
        let entries = vec![
            entry("cmd_recent", "/repo", None, Some(0), now - 10),
            entry("cmd_old", "/repo", None, Some(0), now - 1_000_000),
        ];
        let ranked = rank(&entries, "/repo", None, now, None, 10);
        assert_eq!(ranked[0].command, "cmd_recent");
    }

    #[test]
    fn failed_commands_are_discounted_not_dropped() {
        let now = 1_000_000;
        let entries = vec![
            entry("flaky_cmd", "/repo", None, Some(1), now - 10),
            entry("ok_cmd", "/repo", None, Some(0), now - 10),
        ];
        let ranked = rank(&entries, "/repo", None, now, None, 10);
        assert_eq!(ranked[0].command, "ok_cmd");
        assert!(ranked.iter().any(|r| r.command == "flaky_cmd")); // still present
    }

    #[test]
    fn prefix_filters_results() {
        let now = 1_000_000;
        let entries = vec![
            entry("git status", "/repo", None, Some(0), now - 10),
            entry("cargo build", "/repo", None, Some(0), now - 10),
        ];
        let ranked = rank(&entries, "/repo", None, now, Some("git"), 10);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].command, "git status");
    }

    #[test]
    fn dedupes_by_exact_command_text() {
        let now = 1_000_000;
        let entries = vec![
            entry("git push", "/repo", None, Some(0), now - 10),
            entry("git push", "/repo", None, Some(0), now - 20),
            entry("git push", "/repo", None, Some(0), now - 30),
        ];
        let ranked = rank(&entries, "/repo", None, now, None, 10);
        assert_eq!(ranked.len(), 1);
    }
}
