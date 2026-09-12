use ccusage_core::{Result, cli_error};
use std::{env, path::Path};

pub(super) fn read_key(name: &str, env_file: Option<&Path>) -> Result<String> {
    if let Ok(value) = env::var(name) {
        return validate(value);
    }
    let default_path = env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| ccusage_core::home::home_dir().map(|home| home.join(".config")))
        .map(|dir| dir.join("ccusage/openai.env"));
    if let Some(path) = env_file.or(default_path.as_deref()) {
        let entries = dotenvy::from_path_iter(path)
            .map_err(|_| cli_error("Cannot read OpenAI credentials file"))?;
        let mut value = None;
        for entry in entries {
            let (key, secret) =
                entry.map_err(|_| cli_error("Invalid OpenAI credentials file syntax"))?;
            if key == name {
                value = Some(secret);
            }
        }
        if let Some(value) = value {
            return validate(value);
        }
    }
    Err(cli_error(format!(
        "Missing OpenAI credential environment variable {name}"
    )))
}
fn validate(value: String) -> Result<String> {
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return Err(cli_error(
            "OpenAI credential is empty or contains whitespace",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ccusage_test_support::{EnvVarGuard, fs_fixture};
    #[test]
    fn reads_quoted_dotenv_and_environment_overrides_file() {
        let fixture = fs_fixture!({"openai.env":"CCUSAGE_TEST_OPENAI_CREDENTIAL='fixture-key'\n"});
        let unset = ccusage_test_support::EnvVarsGuard::set_many([(
            "CCUSAGE_TEST_OPENAI_CREDENTIAL",
            None,
        )]);
        let path = fixture.path("openai.env");
        assert_eq!(
            read_key("CCUSAGE_TEST_OPENAI_CREDENTIAL", Some(&path)).unwrap(),
            "fixture-key"
        );
        drop(unset);
        let _set = EnvVarGuard::set("CCUSAGE_TEST_OPENAI_CREDENTIAL", "environment-key");
        assert_eq!(
            read_key("CCUSAGE_TEST_OPENAI_CREDENTIAL", Some(&path)).unwrap(),
            "environment-key"
        );
    }
    #[test]
    fn malformed_dotenv_does_not_expose_secret_in_error() {
        let fixture = fs_fixture!({"openai.env":"KEY='private-example-with-no-closing-quote"});
        let error = read_key(
            "CCUSAGE_TEST_OPENAI_MISSING",
            Some(&fixture.path("openai.env")),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("syntax"));
        assert!(!error.contains("private-example"));
    }
}
