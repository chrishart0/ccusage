//! Run the same local source discovery on a remote machine, without recursion.
use super::types::{AgentRows, AllRow};
use crate::{
    Result,
    cli::{AgentReportKind, SharedArgs},
    cli_error,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) fn load(host: &str, kind: AgentReportKind, shared: &SharedArgs) -> Result<AgentRows> {
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
        &remote_command(kind, shared),
    ]);
    let bytes = collect(&mut command, Duration::from_secs(120))
        .map_err(|error| cli_error(format!("SSH collection from {host} failed: {error}. Install this fork as ~/.local/bin/ccusage-fork on the server")))?;
    parse(&bytes, host, kind)
        .map_err(|error| cli_error(format!("Invalid usage from SSH server {host}: {error}")))
}
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn remote_command(kind: AgentReportKind, shared: &SharedArgs) -> String {
    let report = if kind == AgentReportKind::Session {
        "session"
    } else {
        "daily"
    };
    // Explicit empty config and opt-outs prevent collection loops and account duplication.
    let mut command = format!(
        "exec \"$HOME/.local/bin/ccusage-fork\" {report} --json --by-agent --config /dev/null --no-ssh --no-openai --offline"
    );
    let mode = match shared.mode {
        crate::cli::CostMode::Auto => "auto",
        crate::cli::CostMode::Calculate => "calculate",
        crate::cli::CostMode::Display => "display",
    };
    command.push_str(&format!(" --mode {mode}"));
    for (flag, value) in [
        ("--since", shared.since.as_deref()),
        ("--until", shared.until.as_deref()),
        ("--timezone", shared.timezone.as_deref()),
    ] {
        if let Some(value) = value {
            command.push_str(&format!(" {flag} {}", quote(value)));
        }
    }
    command
}
fn collect(command: &mut Command, timeout: Duration) -> Result<Vec<u8>> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    const LIMIT: u64 = 64 * 1024 * 1024;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(25)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(cli_error(if result.is_err() {
                    "Cannot wait for SSH collector"
                } else {
                    "SSH collection timed out"
                }));
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| cli_error("SSH output reader failed"))??;
    let status = status?;
    if !status.success() {
        return Err(cli_error(format!("ssh exited with {status}")));
    }
    if bytes.len() as u64 > LIMIT {
        return Err(cli_error("SSH usage exceeds 64 MiB"));
    }
    Ok(bytes)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteRow {
    agent: String,
    models_used: Vec<String>,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    total_tokens: u64,
    total_cost: f64,
    #[serde(default)]
    model_breakdowns: Vec<crate::ModelBreakdown>,
}
fn parse(bytes: &[u8], host: &str, kind: AgentReportKind) -> Result<AgentRows> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| cli_error("Expected JSON report"))?;
    let key = if kind == AgentReportKind::Session {
        "session"
    } else {
        "daily"
    };
    let periods = value[key]
        .as_array()
        .ok_or_else(|| cli_error("Missing report rows"))?;
    let mut rows = Vec::new();
    for period in periods {
        let label = period["period"]
            .as_str()
            .ok_or_else(|| cli_error("Missing report period"))?;
        let agents = if kind == AgentReportKind::Session {
            vec![period.clone()]
        } else {
            period["agents"]
                .as_array()
                .ok_or_else(|| cli_error("Missing per-agent counters; update the remote fork"))?
                .clone()
        };
        for value in agents {
            let row: RemoteRow = serde_json::from_value(value)
                .map_err(|_| cli_error("Malformed remote agent counters"))?;
            if row.agent.is_empty() || row.agent.chars().any(char::is_control) {
                return Err(cli_error("Invalid remote agent name"));
            }
            let agent: &'static str =
                Box::leak(format!("ssh:{host}:{}", row.agent).into_boxed_str());
            rows.push(AllRow {
                period: if kind == AgentReportKind::Session {
                    format!("ssh:{host}:{label}")
                } else {
                    label.into()
                },
                agent,
                models_used: row.models_used,
                input_tokens: row.input_tokens,
                output_tokens: row.output_tokens,
                cache_creation_tokens: row.cache_creation_tokens,
                cache_read_tokens: row.cache_read_tokens,
                total_tokens: row.total_tokens,
                total_cost: row.total_cost,
                metadata: None,
                metadata_agents: Some(vec![agent]),
                agent_breakdowns: None,
                model_breakdowns: row.model_breakdowns,
            });
        }
    }
    Ok(AgentRows {
        detected: !rows.is_empty(),
        rows,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn collects_multiple_agents_without_readding_combined_total() {
        let row = |agent| json!({"agent":agent,"modelsUsed":[],"inputTokens":10,"outputTokens":2,"cacheCreationTokens":0,"cacheReadTokens":3,"totalTokens":15,"totalCost":0.2});
        let report = json!({"daily":[{"period":"2026-09-12","totalTokens":30,"agents":[row("hermes"),row("codex")]}]});
        let loaded = parse(
            &serde_json::to_vec(&report).unwrap(),
            "crm",
            AgentReportKind::Daily,
        )
        .unwrap();
        assert_eq!(loaded.rows.len(), 2);
        assert_eq!(loaded.rows.iter().map(|r| r.total_tokens).sum::<u64>(), 30);
        assert_eq!(loaded.rows[0].agent, "ssh:crm:hermes");
        assert_eq!(loaded.rows[1].agent, "ssh:crm:codex");
    }
    #[test]
    fn remote_arguments_disable_recursion_and_quote_shell_characters() {
        let shared = SharedArgs {
            timezone: Some("UTC'; echo bad".into()),
            ..Default::default()
        };
        let cmd = remote_command(AgentReportKind::Daily, &shared);
        assert!(cmd.contains("--no-ssh --no-openai"));
        assert!(cmd.ends_with("--timezone 'UTC'\\''; echo bad'"));
    }
    #[test]
    fn failed_and_stalled_collectors_are_errors() {
        let mut command = Command::new("false");
        assert!(collect(&mut command, Duration::from_secs(1)).is_err());
        let mut command = Command::new("sleep");
        command.arg("30");
        assert!(
            collect(&mut command, Duration::from_millis(50))
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
    }
}
