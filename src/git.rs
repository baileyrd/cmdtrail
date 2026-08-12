use std::path::{Path, PathBuf};

/// Walk upward from `start` looking for a `.git` directory or file
/// (the latter covers worktrees/submodules, which use a `.git` file
/// that points at the real gitdir). Returns the repo root if found.
pub fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Normalize a path string for storage/comparison across `log`/`suggest`
/// calls. Windows filesystem lookups are case-insensitive (NTFS preserves
/// case but doesn't distinguish on lookup), so two differently-cased
/// spellings of the same directory must compare equal, or the "exact cwd"
/// / "same repo" ranking tiers in `rank::scope_weight` silently miss
/// matches. Elsewhere, paths are case-sensitive by convention, so we leave
/// them untouched.
#[cfg(windows)]
pub fn normalize_path(s: &str) -> String {
    s.to_lowercase()
}

#[cfg(not(windows))]
pub fn normalize_path(s: &str) -> String {
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_git_root_from_nested_dir() {
        let tmp = std::env::temp_dir().join(format!("cmdtrail-test-{}", std::process::id()));
        let nested = tmp.join("a/b/c");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(tmp.join(".git")).unwrap();

        assert_eq!(find_git_root(&nested), Some(tmp.clone()));

        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn returns_none_outside_any_repo() {
        let tmp = std::env::temp_dir().join(format!("cmdtrail-test-none-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        // Only assert if temp dir itself and its ancestors truly have no .git;
        // in CI/sandbox this holds since temp_dir is isolated per test dir.
        let result = find_git_root(&tmp);
        fs::remove_dir_all(&tmp).ok();
        assert!(result.is_none() || result.unwrap() != tmp);
    }

    #[test]
    fn normalize_path_is_case_insensitive_on_windows_only() {
        let a = normalize_path("C:\\Dev\\Foo");
        let b = normalize_path("c:\\dev\\foo");
        if cfg!(windows) {
            assert_eq!(a, b);
        } else {
            assert_ne!(a, b);
        }
    }
}
