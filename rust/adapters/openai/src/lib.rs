//! Configured OpenAI Platform organizations; billing data, not local sessions.
mod cache;
mod credentials;
mod loader;
mod parser;
pub use loader::load_daily;
pub use parser::PlatformDay;
#[cfg(test)]
use parser::merge_page;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cached_tokens_are_subsets_of_input_and_costs_come_from_billing() {
        let mut days = std::collections::BTreeMap::new();
        merge_page(&mut days, "usage/completions", &json!({"data":[{"start_time":1789171200,"results":[{"input_tokens":1000,"input_cached_tokens":400,"input_cache_write_tokens":100,"output_tokens":200,"model":"gpt-5.5"}]}]})).unwrap();
        merge_page(&mut days, "costs", &json!({"data":[{"start_time":1789171200,"results":[{"amount":{"value":0.42,"currency":"usd"}}]}]})).unwrap();
        let day = days.values().next().unwrap();
        assert_eq!(day.input_tokens, 500);
        assert_eq!(day.cache_read_tokens, 400);
        assert_eq!(day.cache_creation_tokens, 100);
        assert_eq!(day.total_tokens(), 1200);
        assert_eq!(day.total_cost, 0.42);
    }
}
