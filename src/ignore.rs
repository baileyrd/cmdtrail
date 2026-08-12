//! User-defined ignore list so commands carrying secrets never reach the
//! database in the first place — not stored-then-redacted, just never
//! written. Patterns live in `<data dir>/ignore.txt`, one per line:
//!   - a line with no `*` is a case-insensitive substring match against
//!     the whole command text (e.g. `password` matches any command
//!     containing "password" anywhere, any case)
//!   - a line with `*` is a case-insensitive whole-command glob, where
//!     `*` matches any run of characters (e.g. `curl*Authorization*`)
//!   - blank lines and lines starting with `#` are ignored
//!
//! Loading is best-effort and infallible by design: an unreadable or
//! missing ignore file must never block ordinary command logging, so any
//! I/O error here just means "no patterns" rather than propagating.

use std::path::PathBuf;

const TEMPLATE: &str = "\
# cmdtrail ignore patterns — one per line. Commands matching a pattern
# here are never logged (they're skipped before the database write, not
# stored and then filtered out).
#
# A line with no '*' matches any command containing that text, anywhere,
# case-insensitive. Example (uncomment to use):
#   password
#
# A line with '*' is a whole-command glob; '*' matches any run of
# characters. Examples (uncomment to use):
#   curl*Authorization*
#   export*API_KEY=*
#
# Blank lines and lines starting with '#' are ignored.
";

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// Already-lowercased substring to look for anywhere in the command.
    Substring(String),
    /// Already-lowercased glob (contains at least one `*`), matched
    /// against the whole command.
    Glob(String),
}

fn patterns_path() -> Option<PathBuf> {
    crate::db::default_dir().ok().map(|d| d.join("ignore.txt"))
}

/// Load patterns from `<data dir>/ignore.txt`, creating a commented-out
/// template on first run if the file doesn't exist yet. Never fails: any
/// error (can't resolve the data dir, can't read the file, ...) yields an
/// empty pattern list, i.e. "ignore nothing."
pub fn load_patterns() -> Vec<Pattern> {
    let Some(path) = patterns_path() else {
        return Vec::new();
    };
    if !path.exists() {
        let _ = std::fs::write(&path, TEMPLATE);
    }
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    parse_patterns(&content)
}

pub fn parse_patterns(content: &str) -> Vec<Pattern> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let lower = line.to_lowercase();
            if lower.contains('*') {
                Pattern::Glob(lower)
            } else {
                Pattern::Substring(lower)
            }
        })
        .collect()
}

/// Whether `command` matches any pattern (and so must not be logged).
pub fn is_ignored(patterns: &[Pattern], command: &str) -> bool {
    let lower = command.to_lowercase();
    patterns.iter().any(|p| match p {
        Pattern::Substring(needle) => lower.contains(needle.as_str()),
        Pattern::Glob(glob) => glob_match(glob, &lower),
    })
}

/// Classic two-pointer `*`-only glob match (no `?`), operating on `char`s
/// so multi-byte UTF-8 command text compares correctly. `pattern` and
/// `text` are matched in full (implicit anchors at both ends).
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut match_from = 0usize;

    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            match_from = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            match_from += 1;
            ti = match_from;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let patterns = parse_patterns("# comment\n\n  \npassword\n");
        assert_eq!(patterns, vec![Pattern::Substring("password".into())]);
    }

    #[test]
    fn no_star_is_case_insensitive_substring() {
        let patterns = parse_patterns("PassWord");
        assert!(is_ignored(&patterns, "export MY_PASSWORD_IS=hunter2"));
        assert!(!is_ignored(&patterns, "ls -la"));
    }

    #[test]
    fn star_is_whole_command_glob() {
        let patterns = parse_patterns("curl*Authorization*");
        assert!(is_ignored(&patterns, "curl -H 'Authorization: Bearer xyz' https://api.example.com"));
        // glob is anchored to the whole command, not a substring match
        assert!(!is_ignored(&patterns, "echo curl"));
    }

    #[test]
    fn glob_is_case_insensitive() {
        let patterns = parse_patterns("export*api_key=*");
        assert!(is_ignored(&patterns, "EXPORT API_KEY=abc123"));
    }

    #[test]
    fn unmatched_command_is_not_ignored() {
        let patterns = parse_patterns("password\ncurl*Authorization*");
        assert!(!is_ignored(&patterns, "git status"));
    }

    #[test]
    fn empty_pattern_list_ignores_nothing() {
        assert!(!is_ignored(&[], "export API_KEY=abc123"));
    }

    #[test]
    fn glob_match_handles_multiple_stars_and_empty_runs() {
        assert!(glob_match("a*b*c", "aXXbXXc"));
        assert!(glob_match("a*b*c", "abc"));
        assert!(!glob_match("a*b*c", "acb"));
        assert!(glob_match("*", ""));
        assert!(glob_match("*", "anything"));
    }
}
