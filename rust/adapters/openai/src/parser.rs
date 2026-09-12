use ccusage_core::{Result, TimestampMs, cli_error, format_date};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct PlatformDay {
    pub date: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost: f64,
    pub models: BTreeSet<String>,
    #[serde(default)]
    pub model_breakdowns: BTreeMap<String, ccusage_core::ModelBreakdown>,
}
impl PlatformDay {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.cache_creation_tokens)
            .saturating_add(self.cache_read_tokens)
    }
}

pub(super) fn merge_page(
    days: &mut BTreeMap<String, PlatformDay>,
    endpoint: &str,
    page: &Value,
) -> Result<()> {
    let buckets = page["data"]
        .as_array()
        .ok_or_else(|| cli_error("OpenAI returned invalid usage data"))?;
    for bucket in buckets {
        let seconds = bucket["start_time"]
            .as_i64()
            .and_then(|s| s.checked_mul(1000))
            .ok_or_else(|| cli_error("OpenAI returned an invalid bucket time"))?;
        let date = format_date(TimestampMs::from_millis(seconds), Some("UTC"));
        let results = bucket["results"]
            .as_array()
            .ok_or_else(|| cli_error("OpenAI returned invalid bucket results"))?;
        for result in results {
            let day = days.entry(date.clone()).or_insert_with(|| PlatformDay {
                date: date.clone(),
                ..Default::default()
            });
            if endpoint == "costs" {
                if result["amount"]["currency"].as_str() != Some("usd") {
                    return Err(cli_error("OpenAI costs must be reported in USD"));
                }
                let value = &result["amount"]["value"];
                let amount = value
                    .as_f64()
                    .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| cli_error("OpenAI returned an invalid cost amount"))?;
                day.total_cost += amount;
            } else {
                let input = required_count(result, "input_tokens")?;
                let read = optional_count(result, "input_cached_tokens")?;
                let write = optional_count(result, "input_cache_write_tokens")?;
                let uncached = input
                    .checked_sub(read)
                    .and_then(|n| n.checked_sub(write))
                    .ok_or_else(|| cli_error("OpenAI cached tokens exceed total input tokens"))?;
                let output = if endpoint == "usage/embeddings" {
                    0
                } else {
                    required_count(result, "output_tokens")?
                };
                day.input_tokens = day.input_tokens.saturating_add(uncached);
                day.output_tokens = day.output_tokens.saturating_add(output);
                day.cache_read_tokens = day.cache_read_tokens.saturating_add(read);
                day.cache_creation_tokens = day.cache_creation_tokens.saturating_add(write);
                if let Some(model) = result["model"].as_str() {
                    day.models.insert(model.to_string());
                    let detail = day
                        .model_breakdowns
                        .entry(model.to_string())
                        .or_insert_with(|| ccusage_core::ModelBreakdown {
                            model_name: model.to_string(),
                            ..Default::default()
                        });
                    detail.input_tokens = detail.input_tokens.saturating_add(uncached);
                    detail.output_tokens = detail.output_tokens.saturating_add(output);
                    detail.cache_read_tokens = detail.cache_read_tokens.saturating_add(read);
                    detail.cache_creation_tokens =
                        detail.cache_creation_tokens.saturating_add(write);
                }
            }
        }
    }
    Ok(())
}
fn required_count(value: &Value, key: &str) -> Result<u64> {
    value[key]
        .as_u64()
        .ok_or_else(|| cli_error(format!("OpenAI returned an invalid {key} count")))
}
fn optional_count(value: &Value, key: &str) -> Result<u64> {
    if value.get(key).is_none_or(Value::is_null) {
        Ok(0)
    } else {
        required_count(value, key)
    }
}
