//! Wire [sccache](https://github.com/mozilla/sccache) to a Cloudflare R2 bucket.
//!
//! [`Config::from_lookup`] reads the R2 credentials, bucket and endpoint from a variable
//! lookup (normally the process environment). [`Outcome::env`] turns the result into the
//! `NAME=value` pairs sccache needs: `RUSTC_WRAPPER=sccache`, the `SCCACHE_*` and `AWS_*`
//! variables, and `SCCACHE_S3_RW_MODE=READ_ONLY` for a read-only cache. When the backend is
//! not configured (a fork pull request, Dependabot, a repository without the secrets) the
//! outcome is [`Outcome::Disabled`] and the env clears `RUSTC_WRAPPER`, so the build runs
//! without sccache instead of failing.
//!
//! A configured but wrong backend (an endpoint that is not an R2 URL, an invalid bucket
//! name) is an [`Error`]: that is a typo to fix, not a missing secret.

use std::fmt;

/// Variables read for the R2 Access Key ID, first non-empty wins.
pub const ACCESS_KEY_VARS: &[&str] = &["R2_ACCESS_KEY_ID", "AWS_ACCESS_KEY_ID"];
/// Variables read for the R2 Secret Access Key, first non-empty wins.
pub const SECRET_KEY_VARS: &[&str] = &["R2_SECRET_ACCESS_KEY", "AWS_SECRET_ACCESS_KEY"];
/// Variable read for the bucket name.
pub const BUCKET_VAR: &str = "SCCACHE_BUCKET";
/// Variable read for the endpoint, `https://<ACCOUNT_ID>.r2.cloudflarestorage.com`.
pub const ENDPOINT_VAR: &str = "SCCACHE_ENDPOINT";
/// Variable read for read-only mode (`true`, `1`, `yes`, `on`).
pub const READ_ONLY_VAR: &str = "SCCACHE_R2_READ_ONLY";

/// A complete, validated R2 backend. `Debug` hides the credentials.
#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket: String,
    pub endpoint: String,
    pub read_only: bool,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("access_key_id", &"***")
            .field("secret_access_key", &"***")
            .field("bucket", &self.bucket)
            .field("endpoint", &self.endpoint)
            .field("read_only", &self.read_only)
            .finish()
    }
}

/// What to do with sccache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Use the R2 backend.
    Enabled(Config),
    /// Run without sccache; the names of the missing variables.
    Disabled { missing: Vec<&'static str> },
}

/// A backend that is configured but wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Endpoint(String),
    Bucket(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Endpoint(why) => write!(f, "{ENDPOINT_VAR}: {why}"),
            Error::Bucket(why) => write!(f, "{BUCKET_VAR}: {why}"),
        }
    }
}

impl std::error::Error for Error {}

fn first(lookup: &impl Fn(&str) -> Option<String>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|n| {
        lookup(n)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    })
}

/// Parse a boolean flag: `true`, `1`, `yes`, `on` (any case) are true.
pub fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
}

impl Config {
    /// Read the backend from `lookup`. `read_only` forces read-only mode; otherwise
    /// [`READ_ONLY_VAR`] decides.
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
        read_only: bool,
    ) -> Result<Outcome, Error> {
        let access = first(&lookup, ACCESS_KEY_VARS);
        let secret = first(&lookup, SECRET_KEY_VARS);
        let bucket = first(&lookup, &[BUCKET_VAR]);
        let endpoint = first(&lookup, &[ENDPOINT_VAR]);
        if let Some(b) = &bucket {
            validate_bucket(b)?;
        }
        let endpoint = endpoint.map(|e| validate_endpoint(&e)).transpose()?;
        let mut missing = Vec::new();
        if access.is_none() {
            missing.push(ACCESS_KEY_VARS[0]);
        }
        if secret.is_none() {
            missing.push(SECRET_KEY_VARS[0]);
        }
        if bucket.is_none() {
            missing.push(BUCKET_VAR);
        }
        if endpoint.is_none() {
            missing.push(ENDPOINT_VAR);
        }
        let (Some(access_key_id), Some(secret_access_key), Some(bucket), Some(endpoint)) =
            (access, secret, bucket, endpoint)
        else {
            return Ok(Outcome::Disabled { missing });
        };
        let read_only = read_only || lookup(READ_ONLY_VAR).is_some_and(|v| truthy(&v));
        Ok(Outcome::Enabled(Config {
            access_key_id,
            secret_access_key,
            bucket,
            endpoint,
            read_only,
        }))
    }

    /// Read the backend from the process environment.
    pub fn from_env(read_only: bool) -> Result<Outcome, Error> {
        Self::from_lookup(|n| std::env::var(n).ok(), read_only)
    }
}

/// Check an R2 endpoint: `https://<32 hex account id>.r2.cloudflarestorage.com`, optionally
/// with a jurisdiction (`<id>.eu.r2.cloudflarestorage.com`). Returns it without a trailing `/`.
pub fn validate_endpoint(endpoint: &str) -> Result<String, Error> {
    let e = endpoint.trim().trim_end_matches('/');
    let host = e
        .strip_prefix("https://")
        .ok_or_else(|| Error::Endpoint("must start with https://".into()))?;
    if host.contains('/') {
        return Err(Error::Endpoint(
            "must not carry a path (the bucket goes in SCCACHE_BUCKET)".into(),
        ));
    }
    let account = host
        .strip_suffix(".r2.cloudflarestorage.com")
        .ok_or_else(|| Error::Endpoint("host must end in .r2.cloudflarestorage.com".into()))?;
    let id = account.split('.').next().unwrap_or_default();
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Endpoint(
            "account id must be 32 hex characters".into(),
        ));
    }
    Ok(e.to_owned())
}

/// Check an R2 bucket name: 3-63 characters of `a-z`, `0-9` and `-`, not starting or
/// ending with `-`.
pub fn validate_bucket(bucket: &str) -> Result<(), Error> {
    let ok_len = (3..=63).contains(&bucket.len());
    let ok_chars = bucket
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok_len || !ok_chars || bucket.starts_with('-') || bucket.ends_with('-') {
        return Err(Error::Bucket(format!(
            "{bucket:?} is not a valid R2 bucket name (3-63 of a-z, 0-9, -)"
        )));
    }
    Ok(())
}

impl Outcome {
    /// The `(name, value)` pairs to export. Disabled clears `RUSTC_WRAPPER` only.
    pub fn env(&self) -> Vec<(&'static str, String)> {
        match self {
            Outcome::Disabled { .. } => vec![("RUSTC_WRAPPER", String::new())],
            Outcome::Enabled(c) => {
                let mut v = vec![
                    ("RUSTC_WRAPPER", "sccache".to_owned()),
                    ("CARGO_INCREMENTAL", "0".to_owned()),
                    ("SCCACHE_BUCKET", c.bucket.clone()),
                    ("SCCACHE_ENDPOINT", c.endpoint.clone()),
                    ("SCCACHE_REGION", "auto".to_owned()),
                    ("AWS_ACCESS_KEY_ID", c.access_key_id.clone()),
                    ("AWS_SECRET_ACCESS_KEY", c.secret_access_key.clone()),
                ];
                if c.read_only {
                    v.push(("SCCACHE_S3_RW_MODE", "READ_ONLY".to_owned()));
                }
                v
            }
        }
    }

    /// Whether the variable holds a credential that must never be printed.
    pub fn is_secret(name: &str) -> bool {
        matches!(name, "AWS_ACCESS_KEY_ID" | "AWS_SECRET_ACCESS_KEY")
    }

    pub fn enabled(&self) -> bool {
        matches!(self, Outcome::Enabled(_))
    }
}

/// Render `NAME=value` lines (the `$GITHUB_ENV` / dotenv format).
pub fn render(env: &[(&str, String)]) -> String {
    env.iter().map(|(k, v)| format!("{k}={v}\n")).collect()
}

/// Render `export NAME='value'` lines for `eval` in a POSIX shell.
pub fn render_shell(env: &[(&str, String)]) -> String {
    env.iter()
        .map(|(k, v)| format!("export {k}='{}'\n", v.replace('\'', r"'\''")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const EP: &str = "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com";

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |n| m.get(n).cloned()
    }

    fn full() -> Vec<(&'static str, &'static str)> {
        vec![
            ("R2_ACCESS_KEY_ID", "AK"),
            ("R2_SECRET_ACCESS_KEY", "SK"),
            ("SCCACHE_BUCKET", "rust-cache"),
            ("SCCACHE_ENDPOINT", EP),
        ]
    }

    #[test]
    fn enabled_with_everything() {
        let o = Config::from_lookup(lookup(&full()), false).unwrap();
        let env = o.env();
        assert!(env.contains(&("RUSTC_WRAPPER", "sccache".into())));
        assert!(env.contains(&("AWS_SECRET_ACCESS_KEY", "SK".into())));
        assert!(env.contains(&("SCCACHE_REGION", "auto".into())));
        assert!(!env.iter().any(|(k, _)| *k == "SCCACHE_S3_RW_MODE"));
    }

    #[test]
    fn read_only_flag_and_var() {
        let o = Config::from_lookup(lookup(&full()), true).unwrap();
        assert!(
            o.env()
                .contains(&("SCCACHE_S3_RW_MODE", "READ_ONLY".into()))
        );
        let mut p = full();
        p.push((READ_ONLY_VAR, "Yes"));
        let o = Config::from_lookup(lookup(&p), false).unwrap();
        assert!(
            o.env()
                .contains(&("SCCACHE_S3_RW_MODE", "READ_ONLY".into()))
        );
    }

    #[test]
    fn aws_names_are_a_fallback() {
        let p = [
            ("AWS_ACCESS_KEY_ID", "AK"),
            ("AWS_SECRET_ACCESS_KEY", "SK"),
            ("SCCACHE_BUCKET", "rust-cache"),
            ("SCCACHE_ENDPOINT", EP),
        ];
        assert!(Config::from_lookup(lookup(&p), false).unwrap().enabled());
    }

    #[test]
    fn missing_disables_and_clears_wrapper() {
        let o = Config::from_lookup(lookup(&[("SCCACHE_BUCKET", "rust-cache")]), false).unwrap();
        assert_eq!(
            o,
            Outcome::Disabled {
                missing: vec![
                    "R2_ACCESS_KEY_ID",
                    "R2_SECRET_ACCESS_KEY",
                    "SCCACHE_ENDPOINT"
                ]
            }
        );
        assert_eq!(o.env(), vec![("RUSTC_WRAPPER", String::new())]);
    }

    #[test]
    fn empty_values_count_as_missing() {
        let mut p = full();
        p[1] = ("R2_SECRET_ACCESS_KEY", "  ");
        assert!(!Config::from_lookup(lookup(&p), false).unwrap().enabled());
    }

    #[test]
    fn endpoint_rules() {
        assert_eq!(validate_endpoint(&format!("{EP}/")).unwrap(), EP);
        assert!(validate_endpoint(&EP.replace(".r2.", ".eu.r2.")).is_ok());
        assert!(validate_endpoint(&EP.replace("https", "http")).is_err());
        assert!(validate_endpoint(&format!("{EP}/bucket")).is_err());
        assert!(validate_endpoint("https://abc.r2.cloudflarestorage.com").is_err());
        assert!(validate_endpoint("https://s3.amazonaws.com").is_err());
    }

    #[test]
    fn bad_endpoint_is_an_error_even_when_secrets_are_missing() {
        let r = Config::from_lookup(lookup(&[("SCCACHE_ENDPOINT", "http://x")]), false);
        assert!(matches!(r, Err(Error::Endpoint(_))));
    }

    #[test]
    fn bucket_rules() {
        assert!(validate_bucket("rust-cache-1").is_ok());
        for bad in ["ab", "Rust", "-a-b", "a-b-", "a_b", &"a".repeat(64)] {
            assert!(validate_bucket(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn debug_hides_secrets() {
        let Outcome::Enabled(c) = Config::from_lookup(lookup(&full()), false).unwrap() else {
            panic!("expected enabled")
        };
        let s = format!("{c:?}");
        assert!(!s.contains("SK") && !s.contains("\"AK\""));
    }

    #[test]
    fn rendering() {
        let env = vec![("A", "x".to_owned()), ("B", "it's".to_owned())];
        assert_eq!(render(&env), "A=x\nB=it's\n");
        assert_eq!(render_shell(&env), "export A='x'\nexport B='it'\\''s'\n");
    }
}
