//! Short-lived private cache of normalized billing counters, never credentials.
use super::PlatformDay;
use ccusage_cli::OpenAiAccount;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

pub(super) fn path(account: &OpenAiAccount, key: &str, start: i64, end: i64) -> Option<PathBuf> {
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    let identity =
        serde_json::to_vec(&(1, key, &account.project_ids, start, (end - 1) / 86400)).ok()?;
    let digest = Sha256::digest(identity);
    Some(root.join("ccusage/openai").join(format!("{digest:x}.json")))
}
pub(super) fn read(path: &Path) -> Option<Vec<PlatformDay>> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > 64 * 1024 * 1024
        || metadata.modified().ok()?.elapsed().ok()? >= Duration::from_secs(300)
    {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}
pub(super) fn write(path: &Path, days: &[PlatformDay]) -> std::io::Result<()> {
    let parent = path.parent().expect("cache parent");
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let suffix = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = path.with_extension(format!("{}-{suffix}.tmp", std::process::id()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&serde_json::to_vec(days)?)?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_cache_round_trip_and_corruption() {
        let fixture = ccusage_test_support::fs_fixture!({});
        let path = fixture.path("cache/data.json");
        assert!(read(&path).is_none());
        write(
            &path,
            &[PlatformDay {
                date: "2026-09-12".into(),
                input_tokens: 123,
                ..Default::default()
            }],
        )
        .unwrap();
        assert_eq!(read(&path).unwrap()[0].input_tokens, 123);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::write(&path, "invalid").unwrap();
        assert!(read(&path).is_none());
    }
}
