//! Config files never hold secrets: a TOML file with a secret-named key is
//! refused, whatever its value, so a key pasted into a config is caught at
//! load time instead of being committed or synced.

use crate::Error;

/// Refuses `text` when any table, at any depth, has a secret-named key
/// ([`is_secret_key`]). `origin` names the file in the error.
///
/// # Errors
///
/// [`Error::Inline`] naming the dotted key, or [`Error::InvalidToml`].
pub fn reject_inline_secrets(text: &str, origin: &str) -> Result<(), Error> {
    let value: toml::Value = toml::from_str(text).map_err(|e| Error::InvalidToml {
        origin: origin.to_owned(),
        message: e.to_string(),
    })?;
    walk(origin, "", &value)
}

fn walk(origin: &str, path: &str, value: &toml::Value) -> Result<(), Error> {
    match value {
        toml::Value::Table(table) => {
            for (k, v) in table {
                let child = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                if is_secret_key(k) {
                    return Err(Error::Inline {
                        origin: origin.to_owned(),
                        key: child,
                    });
                }
                walk(origin, &child, v)?;
            }
            Ok(())
        }
        toml::Value::Array(items) => items
            .iter()
            .enumerate()
            .try_for_each(|(i, v)| walk(origin, &format!("{path}[{i}]"), v)),
        _ => Ok(()),
    }
}

/// Whether a config key names a secret: `api_key`, `apikey`, `secret_key`,
/// `access_token`, `password`, `client_secret` or anything ending in
/// `_api_key`, with `-` read as `_` and in any case. A key that only points
/// at one (`api_key_env`) is not a secret.
pub fn is_secret_key(name: &str) -> bool {
    let n = name.to_ascii_lowercase().replace('-', "_");
    matches!(
        n.as_str(),
        "api_key"
            | "apikey"
            | "secret_key"
            | "access_token"
            | "refresh_token"
            | "password"
            | "client_secret"
    ) || n.ends_with("_api_key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_and_nested_secrets_are_rejected_with_their_path() {
        let err = reject_inline_secrets("openai_api_key = \"sk-live\"\n", "app.toml")
            .expect_err("inline");
        let msg = err.to_string();
        assert!(
            msg.contains("app.toml") && msg.contains("openai_api_key") && msg.contains("rejected")
        );
        assert!(!msg.contains("sk-live"), "the value never appears");
        let err = reject_inline_secrets("[providers.openai]\napi-key = \"x\"\n", "cfg")
            .expect_err("nested");
        assert!(matches!(err, Error::Inline { key, .. } if key == "providers.openai.api-key"));
        let err = reject_inline_secrets(
            "[[accounts]]\nname = \"a\"\n[[accounts]]\npassword = \"p\"\n",
            "cfg",
        )
        .expect_err("in an array");
        assert!(matches!(err, Error::Inline { key, .. } if key == "accounts[1].password"));
    }

    #[test]
    fn pointers_to_secrets_and_plain_config_pass() {
        reject_inline_secrets(
            "[providers.openai]\napi_key_env = \"OPENAI_API_KEY\"\nmodel = \"gpt\"\n[think]\nmode = \"on\"\n",
            "app.toml",
        )
        .expect("no secret");
        assert!(!is_secret_key("api_key_env"));
        assert!(is_secret_key("Client-Secret"));
    }

    #[test]
    fn invalid_toml_is_an_error() {
        assert!(matches!(
            reject_inline_secrets("= nope", "x"),
            Err(Error::InvalidToml { .. })
        ));
    }
}
