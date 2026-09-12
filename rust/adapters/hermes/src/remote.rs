//! Read only Hermes billing counters over the user's existing SSH connection.

use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::parser::{HermesEntry, read_session_fields};
use crate::{Result, cli_error};
use serde_json::Value;

// Only billing fields cross the connection. SQLite's read transaction includes
// committed WAL data; copying state.db alone would miss active-session usage.
const COLLECT_SCRIPT: &str = r#"
import json, os, sqlite3
from pathlib import Path
homes = os.environ.get('HERMES_HOME', str(Path.home() / '.hermes')).split(',')
rows = []
paths = []
for home in homes:
    root = Path(home.strip()).expanduser()
    candidates = [root / 'state.db', *sorted(root.glob('profiles/*/state.db'))]
    found = [path.resolve() for path in candidates if path.is_file()]
    if not found:
        raise FileNotFoundError('No Hermes state database found in ' + str(root))
    paths.extend(path for path in found if path not in paths)
for path in paths:
    with sqlite3.connect(path.resolve().as_uri() + '?mode=ro', uri=True, timeout=10) as db:
        rows.extend(db.execute('''SELECT id, model, billing_provider, started_at,
            message_count, input_tokens, output_tokens, cache_read_tokens,
            cache_write_tokens, reasoning_tokens, estimated_cost_usd, actual_cost_usd
            FROM sessions WHERE model IS NOT NULL AND TRIM(model) != '' '''))
print(json.dumps(rows, allow_nan=False, separators=(',', ':')))
"#;

pub(super) fn load_entries(host: &str) -> Result<Vec<HermesEntry>> {
    let mut command = Command::new("ssh");
    command.args([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ServerAliveInterval=10",
        "-o",
        "ServerAliveCountMax=2",
        "--",
        host,
        "python3 -",
    ]);
    let bytes = collect(&mut command, Duration::from_secs(60))
        .map_err(|error| cli_error(format!("Hermes SSH collection from {host} failed: {error}")))?;
    parse_rows(&bytes, host)
        .map_err(|error| cli_error(format!("Invalid Hermes usage from {host}: {error}")))
}

fn collect(command: &mut Command, timeout: Duration) -> Result<Vec<u8>> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    const LIMIT: u64 = 64 * 1024 * 1024;
    // Drain both pipes while waiting to avoid deadlocks on large databases/errors.
    let output = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let errors = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.take(65537).read_to_end(&mut bytes).map(|_| bytes)
    });
    let write_result = child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(COLLECT_SCRIPT.as_bytes());
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if start.elapsed() < timeout && write_result.is_ok() => {
                thread::sleep(Duration::from_millis(25))
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                break match result {
                    Err(error) => Err(cli_error(error.to_string())),
                    _ => Err(cli_error(
                        "SSH collection timed out or could not send collector",
                    )),
                };
            }
        }
    };
    let bytes = output
        .join()
        .map_err(|_| cli_error("SSH output reader panicked"))??;
    let errors = errors
        .join()
        .map_err(|_| cli_error("SSH error reader panicked"))??;
    let status = status?;
    if !status.success() {
        return Err(cli_error(format!(
            "ssh exited with {status}: {}",
            String::from_utf8_lossy(&errors).trim()
        )));
    }
    write_result?;
    if bytes.len() as u64 > LIMIT {
        return Err(cli_error("SSH usage exceeds the 64 MiB response limit"));
    }
    Ok(bytes)
}

fn parse_rows(bytes: &[u8], host: &str) -> Result<Vec<HermesEntry>> {
    let rows: Vec<Vec<Value>> = serde_json::from_slice(bytes)?;
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        if row.len() != 12 {
            return Err(cli_error("expected 12 Hermes billing fields"));
        }
        if row
            .iter()
            .any(|value| !matches!(value, Value::Null | Value::String(_) | Value::Number(_)))
        {
            return Err(cli_error("invalid Hermes billing field"));
        }
        if let Some(mut entry) = read_session_fields(
            |index| row[index].as_str().map(str::to_string),
            |index| row[index].as_f64().filter(|value| value.is_finite()),
            |index| {
                row[index].as_u64().unwrap_or_else(|| {
                    row[index]
                        .as_f64()
                        .filter(|value| value.is_finite() && *value > 0.0)
                        .map(|value| value.trunc() as u64)
                        .unwrap_or(0)
                })
            },
        ) {
            entry.session_id = format!("ssh:{host}:{}", entry.session_id);
            if seen.insert(entry.session_id.clone()) {
                entries.push(entry);
            }
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::super::parser::to_loaded_entry;
    use super::*;
    use ccusage_core::PricingMap;

    #[test]
    fn collector_discovers_active_profiles_but_not_snapshots() {
        let fixture = ccusage_test_support::fs_fixture!({});
        std::fs::create_dir_all(fixture.path("profiles/analyst")).unwrap();
        std::fs::create_dir_all(fixture.path("state-snapshots/old")).unwrap();
        let db = sqlite::open(fixture.path("profiles/analyst/state.db")).unwrap();
        db.execute("CREATE TABLE sessions (id TEXT, model TEXT, billing_provider TEXT, started_at REAL, message_count INTEGER, input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER, reasoning_tokens INTEGER, estimated_cost_usd REAL, actual_cost_usd REAL); INSERT INTO sessions VALUES ('profile-session', 'gpt-5.5', 'openai', 1789171200, 3, 100, 20, 50, 10, 5, 0.25, NULL);").unwrap();
        std::fs::write(
            fixture.path("state-snapshots/old/state.db"),
            "not a database",
        )
        .unwrap();
        let mut command = Command::new("python3");
        command.arg("-").env("HERMES_HOME", fixture.path(""));
        let bytes = collect(&mut command, Duration::from_secs(5)).unwrap();
        let rows = parse_rows(&bytes, "crm").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id, "ssh:crm:profile-session");
    }

    #[test]
    fn collector_reads_live_wal_and_omits_conversation_content() {
        let fixture = ccusage_test_support::fs_fixture!({});
        let db = sqlite::open(fixture.path("state.db")).unwrap();
        db.execute("PRAGMA journal_mode=WAL; CREATE TABLE sessions (id TEXT, model TEXT, billing_provider TEXT, started_at REAL, message_count INTEGER, input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER, reasoning_tokens INTEGER, estimated_cost_usd REAL, actual_cost_usd REAL, system_prompt TEXT); INSERT INTO sessions VALUES ('live', 'gpt-5.5', 'openai', 1789171200, 3, 100, 20, 50, 10, 5, 0.25, NULL, 'private conversation');").unwrap();
        let mut command = Command::new("python3");
        command.arg("-").env("HERMES_HOME", fixture.path(""));
        let bytes = collect(&mut command, Duration::from_secs(5)).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("private conversation"));
        let rows = parse_rows(&bytes, "crm").unwrap();
        assert_eq!(rows.len(), 1);
        let row = to_loaded_entry(
            rows.into_iter().next().unwrap(),
            None,
            &PricingMap::load_embedded(),
        );
        assert_eq!(row.cost, 0.25);
        assert_eq!(row.data.message.usage.input_tokens, 100);
    }

    #[test]
    fn collector_reports_missing_database() {
        let fixture = ccusage_test_support::fs_fixture!({});
        let mut command = Command::new("python3");
        command.arg("-").env("HERMES_HOME", fixture.path(""));
        assert!(
            collect(&mut command, Duration::from_secs(5))
                .unwrap_err()
                .to_string()
                .contains("exited")
        );
        assert!(!fixture.path("state.db").exists());
    }

    #[test]
    fn collector_terminates_unresponsive_process() {
        let mut command = Command::new("python3");
        command.args(["-c", "import time; time.sleep(30)"]);
        let start = Instant::now();
        assert!(
            collect(&mut command, Duration::from_millis(100))
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn repeated_remote_sessions_count_once_per_host() {
        let payload = br#"[["s","gpt-5.5","openai",1789171200,3,100,20,50,10,5,0.25,null],["s","gpt-5.5","openai",1789171200,3,100,20,50,10,5,0.25,null]]"#;
        let rows = parse_rows(payload, "crm").unwrap();
        assert_eq!(rows.len(), 1);
        assert_ne!(
            rows[0].session_id,
            parse_rows(payload, "backup").unwrap()[0].session_id
        );
    }

    #[test]
    fn remote_rows_preserve_tokens_and_namespace_sessions() {
        let payload =
            br#"[["session-1","gpt-5.5","openai",1789171200,3,100,20,50,10,5,0.25,null]]"#;
        let rows = parse_rows(payload, "crm").unwrap();
        let row = to_loaded_entry(
            rows.into_iter().next().unwrap(),
            None,
            &PricingMap::load_embedded(),
        );
        assert_eq!(row.session_id.as_ref(), "ssh:crm:session-1");
        assert_eq!(row.data.message.usage.input_tokens, 100);
        assert_eq!(row.data.message.usage.output_tokens, 20);
        assert_eq!(row.data.message.usage.cache_read_input_tokens, 50);
        assert_eq!(row.extra_total_tokens, 5);
        assert_eq!(row.cost, 0.25);
    }

    #[test]
    fn malformed_remote_data_fails_instead_of_reporting_zero() {
        for payload in [b"not json".as_slice(), b"{}", b"[[1,2]]"] {
            assert!(parse_rows(payload, "crm").is_err());
        }
    }
}
