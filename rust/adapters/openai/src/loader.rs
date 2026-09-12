use super::{PlatformDay, credentials::read_key, parser::merge_page};
use ccusage_cli::{OpenAiAccount, SharedArgs, normalize_date_bound};
use ccusage_core::{Result, cli_error, parse_ts_timestamp, utc_now};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    time::Duration,
};

pub fn load_daily(account: &OpenAiAccount, shared: &SharedArgs) -> Result<Vec<PlatformDay>> {
    load_inner(account, shared)
        .map_err(|error| cli_error(format!("OpenAI account '{}': {error}", account.name)))
}
fn load_inner(account: &OpenAiAccount, shared: &SharedArgs) -> Result<Vec<PlatformDay>> {
    let start = date_seconds(shared.since.as_deref().unwrap_or(&account.since))?
        .max(date_seconds(&account.since)?);
    let now = utc_now().as_millis() / 1000;
    let end = shared
        .until
        .as_deref()
        .map(date_seconds)
        .transpose()?
        .map(|s| s + 86400)
        .unwrap_or(now)
        .min(now);
    if start >= end {
        return Ok(Vec::new());
    }
    let key = read_key(&account.key_env, shared.openai_env_file.as_deref())?;
    let cache_path = super::cache::path(account, &key, start, end);
    if !shared.refresh_openai
        && let Some(days) = cache_path.as_deref().and_then(super::cache::read)
    {
        return Ok(days);
    }
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .new_agent();
    let days = collect_sources(shared.single_thread, |endpoint| {
        let mut days = BTreeMap::new();
        collect_pages(&mut days, endpoint, |page| {
            request_page(&agent, endpoint, &key, account, start, end, page)
        })?;
        Ok(days)
    })?;
    let days: Vec<_> = days
        .into_values()
        .filter(|day| day.total_tokens() != 0 || day.total_cost != 0.0)
        .collect();
    if let Some(path) = cache_path {
        let _ = super::cache::write(&path, &days);
    }
    Ok(days)
}

// Three independent endpoints per account; cursor pages remain sequential.
// Merge in endpoint order so scheduling cannot change report totals.
fn collect_sources(
    single_thread: bool,
    collect: impl Fn(&str) -> Result<BTreeMap<String, PlatformDay>> + Sync,
) -> Result<BTreeMap<String, PlatformDay>> {
    let endpoints = ["usage/completions", "usage/embeddings", "costs"];
    let sources = if single_thread {
        endpoints
            .into_iter()
            .map(&collect)
            .collect::<Result<Vec<_>>>()?
    } else {
        std::thread::scope(|scope| {
            let workers: Vec<_> = endpoints
                .into_iter()
                .map(|endpoint| {
                    let collect = &collect;
                    scope.spawn(move || collect(endpoint))
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| {
                    worker
                        .join()
                        .map_err(|_| cli_error("OpenAI collector thread failed"))?
                })
                .collect::<Result<Vec<_>>>()
        })?
    };
    let mut days: BTreeMap<String, PlatformDay> = BTreeMap::new();
    for source in sources {
        for (date, day) in source {
            let total = days.entry(date).or_insert_with(|| PlatformDay {
                date: day.date,
                ..Default::default()
            });
            total.input_tokens = total.input_tokens.saturating_add(day.input_tokens);
            total.output_tokens = total.output_tokens.saturating_add(day.output_tokens);
            total.cache_creation_tokens = total
                .cache_creation_tokens
                .saturating_add(day.cache_creation_tokens);
            total.cache_read_tokens = total
                .cache_read_tokens
                .saturating_add(day.cache_read_tokens);
            total.total_cost += day.total_cost;
            total.models.extend(day.models);
        }
    }
    Ok(days)
}

pub(super) fn collect_pages(
    days: &mut BTreeMap<String, PlatformDay>,
    endpoint: &str,
    mut fetch: impl FnMut(Option<&str>) -> Result<Value>,
) -> Result<()> {
    let mut cursor: Option<String> = None;
    let mut seen = HashSet::new();
    for _ in 0..512 {
        let value = fetch(cursor.as_deref())?;
        merge_page(days, endpoint, &value)?;
        match value["has_more"].as_bool() {
            Some(false) => return Ok(()),
            Some(true) => {
                let next = value["next_page"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| cli_error("OpenAI pagination omitted its cursor"))?;
                if !seen.insert(next.to_string()) {
                    return Err(cli_error("OpenAI pagination repeated its cursor"));
                }
                cursor = Some(next.to_string());
            }
            None => return Err(cli_error("OpenAI pagination response is invalid")),
        }
    }
    Err(cli_error(
        "OpenAI pagination exceeded 512 pages; narrow the date range",
    ))
}
fn request_page(
    agent: &ureq::Agent,
    endpoint: &str,
    key: &str,
    account: &OpenAiAccount,
    start: i64,
    end: i64,
    cursor: Option<&str>,
) -> Result<Value> {
    // Credentials are only sent to the fixed official API, never a configurable host.
    let url = format!("https://api.openai.com/v1/organization/{endpoint}");
    for attempt in 0..3 {
        let mut request = agent
            .get(&url)
            .header("Authorization", format!("Bearer {key}"))
            .query("start_time", start.to_string())
            .query("end_time", end.to_string())
            .query("bucket_width", "1d")
            .query("limit", if endpoint == "costs" { "180" } else { "31" });
        if endpoint != "costs" {
            request = request.query("group_by", "model");
        }
        for project in &account.project_ids {
            request = request.query("project_ids", project);
        }
        if let Some(cursor) = cursor {
            request = request.query("page", cursor);
        }
        // Never surface response bodies or HTTP debug data: they can contain account details.
        let mut response = match request.call() {
            Ok(response) => response,
            Err(_) if attempt < 2 => {
                std::thread::sleep(Duration::from_secs(1 << attempt));
                continue;
            }
            Err(error) => {
                let reason = match error {
                    ureq::Error::Timeout(_) => "request timed out",
                    _ => "network or TLS error",
                };
                return Err(cli_error(format!("OpenAI {endpoint}: {reason}")));
            }
        };
        let status = response.status().as_u16();
        if (status == 429 || status >= 500) && attempt < 2 {
            let wait = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(1 << attempt);
            if wait > 30 {
                return Err(cli_error("OpenAI rate limited collection; retry later"));
            }
            std::thread::sleep(Duration::from_secs(wait));
            continue;
        }
        if status != 200 {
            return Err(cli_error(format!(
                "OpenAI HTTP {status}; check the admin key and organization usage/cost permissions"
            )));
        }
        let text = response
            .body_mut()
            .with_config()
            .limit(16 * 1024 * 1024)
            .read_to_string()
            .map_err(|_| cli_error("Cannot read OpenAI response"))?;
        return serde_json::from_str(&text).map_err(|_| cli_error("OpenAI returned invalid JSON"));
    }
    Err(cli_error("OpenAI request retries exhausted"))
}
fn date_seconds(value: &str) -> Result<i64> {
    let value = normalize_date_bound(value)
        .ok_or_else(|| cli_error("Invalid OpenAI history start date"))?;
    parse_ts_timestamp(&format!(
        "{}-{}-{}T00:00:00Z",
        &value[..4],
        &value[4..6],
        &value[6..]
    ))
    .map(|time| time.as_millis() / 1000)
    .ok_or_else(|| cli_error("Invalid OpenAI history date"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn independent_endpoints_run_concurrently_and_keep_identical_totals() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let started = AtomicUsize::new(0);
        let fixture = |endpoint: &str| {
            let results = match endpoint {
                "usage/completions" => {
                    json!([{"input_tokens":100,"input_cached_tokens":30,"output_tokens":20,"model":"chat"}])
                }
                "usage/embeddings" => json!([{"input_tokens":10,"model":"embedding"}]),
                _ => json!([{"amount":{"value":0.42,"currency":"usd"}}]),
            };
            let mut days = BTreeMap::new();
            merge_page(
                &mut days,
                endpoint,
                &json!({"data":[{"start_time":1789171200,"results":results}]}),
            )?;
            Ok(days)
        };
        let parallel = collect_sources(false, |endpoint| {
            started.fetch_add(1, Ordering::SeqCst);
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while started.load(Ordering::SeqCst) < 3 {
                assert!(
                    std::time::Instant::now() < deadline,
                    "endpoints must start concurrently"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            fixture(endpoint)
        })
        .unwrap();
        let sequential = collect_sources(true, fixture).unwrap();
        assert_eq!(
            serde_json::to_value(&parallel).unwrap(),
            serde_json::to_value(sequential).unwrap()
        );
        let day = parallel.values().next().unwrap();
        assert_eq!(day.total_tokens(), 130);
        assert_eq!(day.total_cost, 0.42);
        assert_eq!(day.models.len(), 2);
    }

    #[test]
    fn parallel_source_failure_fails_the_whole_report() {
        let result = collect_sources(false, |endpoint| {
            if endpoint == "costs" {
                Err(cli_error("billing unavailable"))
            } else {
                Ok(BTreeMap::new())
            }
        });
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("billing unavailable")
        );
    }

    #[test]
    fn follows_all_pages_including_empty_history() {
        let mut calls = 0;
        let mut days = BTreeMap::new();
        collect_pages(&mut days, "usage/embeddings", |cursor| {
            calls += 1;
            if calls == 1 { assert!(cursor.is_none()); Ok(json!({"data":[],"has_more":true,"next_page":"older"})) }
            else { assert_eq!(cursor,Some("older")); Ok(json!({"data":[{"start_time":1789171200,"results":[{"input_tokens":70,"model":"embedding"}]}],"has_more":false})) }
        }).unwrap();
        assert_eq!(calls, 2);
        assert_eq!(days.values().next().unwrap().total_tokens(), 70);
    }
    #[test]
    fn repeated_cursor_fails_instead_of_counting_forever() {
        let error = collect_pages(&mut BTreeMap::new(), "costs", |_| {
            Ok(json!({"data":[],"has_more":true,"next_page":"same"}))
        })
        .unwrap_err();
        assert!(error.to_string().contains("repeated"));
    }
    #[test]
    fn cost_only_days_and_negative_adjustments_are_preserved() {
        let mut days = BTreeMap::new();
        merge_page(&mut days,"costs", &json!({"data":[{"start_time":1789171200,"results":[{"amount":{"value":"1.25","currency":"usd"}},{"amount":{"value":-0.25,"currency":"usd"}}]}]})).unwrap();
        let day = days.values().next().unwrap();
        assert_eq!(day.total_cost, 1.0);
        assert_eq!(day.total_tokens(), 0);
    }
}
