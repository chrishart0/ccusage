use crate::config_schema::OpenAiConfig;
use ccusage_cli::{OpenAiAccount, normalize_date_bound};
use serde_json::Value;
use std::path::PathBuf;

pub(crate) fn parse(
    value: Option<&Value>,
) -> Result<(Vec<OpenAiAccount>, Option<PathBuf>), String> {
    let Some(raw) = value.and_then(|value| value.get("openai")) else {
        return Ok((Vec::new(), None));
    };
    let config: OpenAiConfig = serde_json::from_value(raw.clone()).map_err(|_| "Invalid OpenAI config: use accounts with name/keyEnv, optional since/projectIds, and an optional envFile".to_string())?;
    let mut accounts: Vec<OpenAiAccount> = Vec::new();
    for account in config.accounts {
        if account.name.trim().is_empty() || account.name.chars().any(char::is_control) {
            return Err(
                "OpenAI account names must be nonempty and contain no control characters"
                    .to_string(),
            );
        }
        let mut bytes = account.key_env.bytes();
        if !bytes
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
            || !bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return Err(
                "OpenAI keyEnv must be an environment variable name, not a key value".to_string(),
            );
        }
        let since = normalize_date_bound(
            account
                .since
                .as_deref()
                .or(config.since.as_deref())
                .unwrap_or("2020-01-01"),
        )
        .ok_or_else(|| "Invalid OpenAI history since date; use YYYY-MM-DD".to_string())?;
        let mut project_ids = account.project_ids;
        project_ids.sort();
        project_ids.dedup();
        if project_ids
            .iter()
            .any(|id| id.is_empty() || id.chars().any(char::is_control))
        {
            return Err("OpenAI project IDs must be nonempty".to_string());
        }
        for previous in &accounts {
            if previous.name == account.name {
                return Err("OpenAI account names must be unique".to_string());
            }
            if previous.key_env == account.key_env
                && (previous.project_ids.is_empty()
                    || project_ids.is_empty()
                    || previous
                        .project_ids
                        .iter()
                        .any(|id| project_ids.contains(id)))
            {
                return Err("OpenAI accounts using the same keyEnv must have disjoint projectIds to avoid double-counting".to_string());
            }
        }
        accounts.push(OpenAiAccount {
            name: account.name,
            key_env: account.key_env,
            since,
            project_ids,
        });
    }
    let env_file = config.env_file.map(|path| {
        if let Some(rest) = path.strip_prefix("~/") {
            ccusage_core::home::home_dir()
                .map(|home| home.join(rest))
                .unwrap_or_else(|| PathBuf::from(path))
        } else {
            PathBuf::from(path)
        }
    });
    Ok((accounts, env_file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn account_names_reference_keys_without_loading_secret_values() {
        let value = json!({"openai":{"accounts":[{"name":"Prouct 1","keyEnv":"OPENAI_ADMIN_KEY_1"},{"name":"product_2","keyEnv":"OPENAI_ADMIN_KEY_2"}]}});
        let (accounts, _) = parse(Some(&value)).unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].name, "Prouct 1");
        assert_eq!(accounts[0].since, "20200101");
    }
    #[test]
    fn rejects_duplicate_accounts_and_overlapping_projects() {
        for accounts in [
            json!([{"name":"one","keyEnv":"KEY"},{"name":"two","keyEnv":"KEY"}]),
            json!([{"name":"one","keyEnv":"KEY_1"},{"name":"one","keyEnv":"KEY_2"}]),
        ] {
            assert!(parse(Some(&json!({"openai":{"accounts":accounts}}))).is_err());
        }
    }
}
