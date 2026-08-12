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
}
