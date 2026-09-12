use std::{collections::HashSet, env, path::PathBuf};

use crate::Result;

const HERMES_HOME_ENV: &str = "HERMES_HOME";

pub(super) fn hermes_state_db_paths() -> Result<Vec<PathBuf>> {
    let homes = if let Ok(paths) = env::var(HERMES_HOME_ENV) {
        paths
            .split(',')
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    } else {
        let home =
            crate::home::home_dir().ok_or_else(|| crate::cli_error("home directory is not set"))?;
        vec![home.join(".hermes")]
    };
    Ok(discover_paths(homes))
}
fn discover_paths(homes: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for home in homes {
        let mut candidates = vec![home.join("state.db")];
        if let Ok(profiles) = std::fs::read_dir(home.join("profiles")) {
            let mut profiles: Vec<_> = profiles
                .flatten()
                .map(|entry| entry.path().join("state.db"))
                .collect();
            profiles.sort();
            candidates.extend(profiles);
        }
        for path in candidates.into_iter().filter(|path| path.is_file()) {
            let path = path.canonicalize().unwrap_or(path);
            if seen.insert(path.clone()) {
                paths.push(path);
            }
        }
    }
    paths
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_profiles_once_and_excludes_snapshots() {
        let fixture = ccusage_test_support::fs_fixture!({
            "state.db":"", "profiles/analyst/state.db":"", "state-snapshots/old/state.db":""
        });
        let paths = discover_paths(vec![fixture.path(""), fixture.path("profiles/analyst")]);
        assert_eq!(paths.len(), 2);
        assert!(
            paths
                .iter()
                .any(|path| path.ends_with("profiles/analyst/state.db"))
        );
        assert!(
            !paths
                .iter()
                .any(|path| path.to_string_lossy().contains("state-snapshots"))
        );
    }
}
