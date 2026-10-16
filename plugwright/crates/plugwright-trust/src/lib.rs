//! Detached minisign (ed25519) signatures for plugin artifacts and the [`TrustPolicy`] that
//! decides, before an artifact is mapped or compiled, whether it may load.
//!
//! The signature of `lib/foo.so` is `lib/foo.so.minisig`, as written by `minisign -S -m
//! lib/foo.so` or by [`sign_file`] (feature `sign`). Only prehashed signatures (the minisign
//! default since 0.9) are accepted, so an artifact is streamed, never read whole into memory.

use std::fmt;
use std::fs::File;
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use minisign_verify::{Error as SigError, PublicKey, Signature, StreamVerifier};
use plugwright_core::error::{PluginError, Result, io_err};
use serde::{Deserialize, Serialize};

#[cfg(feature = "sign")]
pub use minisign;

/// A minisign signature is under 400 bytes; a bigger file is refused before it is read whole.
const MAX_SIGNATURE_BYTES: u64 = 4096;

/// What happens to an artifact that carries no signature a trusted key could check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Unsigned {
    /// Refuse to load it.
    Deny,
    /// Load it and log a warning.
    Warn,
    /// Load it silently.
    Allow,
}

/// Which keys a plugin source accepts (`[settings.trust]`, `[plugin.trust]` in the manifest).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrustPolicy {
    /// minisign public keys in base64, as on the second line of `minisign.pub`.
    pub keys: Vec<String>,
    /// Unset means `deny` when `keys` is not empty and `allow` otherwise.
    pub unsigned: Option<Unsigned>,
}

/// A successful check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Signed by a trusted key; carries the signature's trusted comment.
    Signed { trusted_comment: String },
    /// No usable signature, and the policy lets the artifact through.
    Unsigned,
}

/// Where the detached signature of `artifact` lives: `<artifact>.minisig`.
pub fn signature_path(artifact: &Path) -> PathBuf {
    let mut s = artifact.as_os_str().to_owned();
    s.push(".minisig");
    s.into()
}

impl TrustPolicy {
    /// The mode applied to unsigned artifacts once defaults are filled in.
    pub fn unsigned_mode(&self) -> Unsigned {
        self.unsigned.unwrap_or(if self.keys.is_empty() { Unsigned::Allow } else { Unsigned::Deny })
    }

    /// Reject keys that do not parse and a `deny` nothing could satisfy, so a typo fails when the
    /// manifest is read instead of on every load.
    pub fn validate(&self) -> std::result::Result<(), String> {
        for k in &self.keys {
            PublicKey::from_base64(k).map_err(|e| format!("trust key `{k}` is not a minisign public key: {e}"))?;
        }
        if self.keys.is_empty() && self.unsigned == Some(Unsigned::Deny) {
            return Err("trust: `unsigned = \"deny\"` needs at least one key in `keys`".into());
        }
        Ok(())
    }

    /// Check `artifact` of plugin `id` against `<artifact>.minisig`. A signature that is
    /// present but malformed, untrusted or not matching is an error in every mode; only a
    /// missing one falls back to [`TrustPolicy::unsigned_mode`].
    pub fn verify(&self, id: &str, artifact: &Path) -> Result<Verdict> {
        if self.keys.is_empty() {
            // Nothing could vouch for the artifact, whatever lies next to it.
            return self.unsigned(id, artifact, "no trusted keys");
        }
        let sig_path = signature_path(artifact);
        let fail = |reason: String| PluginError::Signature { path: artifact.to_owned(), reason };
        let Some(text) = read_signature(&sig_path)? else {
            return self.unsigned(id, artifact, "no signature");
        };
        let sig = Signature::decode(&text).map_err(|e| fail(format!("{}: {e}", sig_path.display())))?;
        for k in &self.keys {
            let key = PublicKey::from_base64(k).map_err(|e| fail(format!("trust key `{k}`: {e}")))?;
            match check(&key, &sig, artifact) {
                Ok(()) => return Ok(Verdict::Signed { trusted_comment: sig.trusted_comment().to_owned() }),
                Err(SigError::UnexpectedKeyId) => continue,
                Err(SigError::IoError(e)) => return Err(io_err(artifact)(e)),
                Err(SigError::InvalidSignature) => {
                    return Err(fail(
                        "does not match its signature: the artifact or the signature was modified".into(),
                    ));
                }
                Err(e) => return Err(fail(format!("{}: {e}", sig_path.display()))),
            }
        }
        // The signature's comments are attacker-controlled text, so they are not echoed.
        Err(fail(format!("signed by a key that plugin `{id}` does not trust")))
    }

    fn unsigned(&self, id: &str, artifact: &Path, why: &str) -> Result<Verdict> {
        match self.unsigned_mode() {
            Unsigned::Allow => Ok(Verdict::Unsigned),
            Unsigned::Warn => {
                tracing::warn!(plugin = id, artifact = %artifact.display(), "{why}: loading an unsigned artifact");
                Ok(Verdict::Unsigned)
            }
            Unsigned::Deny => Err(PluginError::Signature {
                path: artifact.to_owned(),
                reason: format!("{why}, and the trust policy of plugin `{id}` refuses unsigned artifacts"),
            }),
        }
    }
}

impl fmt::Display for TrustPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mode = match self.unsigned_mode() {
            Unsigned::Deny => "refused",
            Unsigned::Warn => "warned about",
            Unsigned::Allow => "allowed",
        };
        write!(f, "unsigned {mode}, ")?;
        if self.keys.is_empty() {
            return write!(f, "no keys");
        }
        let short: Vec<&str> = self.keys.iter().map(|k| k.get(..12).unwrap_or(k)).collect();
        write!(f, "keys {}…", short.join("…, "))
    }
}

/// `None` when there is no signature file.
fn read_signature(path: &Path) -> Result<Option<String>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_err(path)(e)),
    };
    let mut text = String::new();
    file.take(MAX_SIGNATURE_BYTES + 1).read_to_string(&mut text).map_err(io_err(path))?;
    if text.len() as u64 > MAX_SIGNATURE_BYTES {
        return Err(PluginError::Signature {
            path: path.to_owned(),
            reason: format!("larger than {MAX_SIGNATURE_BYTES} bytes; not a minisign signature"),
        });
    }
    Ok(Some(text))
}

fn check(key: &PublicKey, sig: &Signature, artifact: &Path) -> std::result::Result<(), SigError> {
    struct Sink<'a>(StreamVerifier<'a>);
    impl Write for Sink<'_> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.update(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut sink = Sink(key.verify_stream(sig)?);
    io::copy(&mut File::open(artifact)?, &mut sink)?;
    sink.0.finalize()
}

/// Sign `artifact` with `sk` and write `<artifact>.minisig`; returns the signature path.
/// `trusted_comment` defaults to minisign's `timestamp:<unix seconds>`.
#[cfg(feature = "sign")]
pub fn sign_file(artifact: &Path, sk: &minisign::SecretKey, trusted_comment: Option<&str>) -> Result<PathBuf> {
    let file = File::open(artifact).map_err(io_err(artifact))?;
    let sig = minisign::sign(None, sk, file, trusted_comment, None)
        .map_err(|e| PluginError::Signature { path: artifact.to_owned(), reason: format!("cannot sign: {e}") })?;
    let path = signature_path(artifact);
    std::fs::write(&path, sig.into_string()).map_err(io_err(&path))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use minisign::KeyPair;

    struct Fixture {
        _dir: tempfile::TempDir,
        artifact: PathBuf,
        signer: KeyPair,
    }

    fn signed(bytes: &[u8]) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("libp.so");
        std::fs::write(&artifact, bytes).unwrap();
        let signer = KeyPair::generate_unencrypted_keypair().unwrap();
        sign_file(&artifact, &signer.sk, Some("plugin p")).unwrap();
        Fixture { _dir: dir, artifact, signer }
    }

    fn key(kp: &KeyPair) -> String {
        kp.pk.to_base64()
    }

    fn policy(keys: &[String], unsigned: Option<Unsigned>) -> TrustPolicy {
        TrustPolicy { keys: keys.to_vec(), unsigned }
    }

    fn reason(r: Result<Verdict>) -> String {
        match r {
            Err(PluginError::Signature { reason, .. }) => reason,
            other => panic!("expected a signature error, got {other:?}"),
        }
    }

    #[test]
    fn a_valid_signature_from_a_trusted_key_passes_in_every_mode() {
        let f = signed(b"\x7fELF plugin bytes");
        assert_eq!(signature_path(&f.artifact), f.artifact.with_file_name("libp.so.minisig"));
        let other = key(&KeyPair::generate_unencrypted_keypair().unwrap());
        for mode in [None, Some(Unsigned::Deny), Some(Unsigned::Warn), Some(Unsigned::Allow)] {
            // The signing key need not be the first one listed.
            let p = policy(&[other.clone(), key(&f.signer)], mode);
            assert_eq!(p.verify("p", &f.artifact).unwrap(), Verdict::Signed { trusted_comment: "plugin p".into() });
        }
    }

    #[test]
    fn a_signature_from_an_untrusted_key_is_refused_in_every_mode() {
        let f = signed(b"bytes");
        let stranger = key(&KeyPair::generate_unencrypted_keypair().unwrap());
        for mode in [Unsigned::Deny, Unsigned::Warn, Unsigned::Allow] {
            let err = reason(policy(std::slice::from_ref(&stranger), Some(mode)).verify("p", &f.artifact));
            assert_eq!(err, "signed by a key that plugin `p` does not trust");
        }
    }

    #[test]
    fn a_tampered_artifact_or_signature_is_refused() {
        let f = signed(b"original bytes");
        let p = policy(&[key(&f.signer)], Some(Unsigned::Allow));
        std::fs::write(&f.artifact, b"original bytes, patched").unwrap();
        assert!(reason(p.verify("p", &f.artifact)).contains("does not match its signature"));

        // A rewritten trusted comment breaks the global signature.
        std::fs::write(&f.artifact, b"original bytes").unwrap();
        let sig = signature_path(&f.artifact);
        let text = std::fs::read_to_string(&sig).unwrap().replace("plugin p", "plugin q");
        std::fs::write(&sig, text).unwrap();
        assert!(reason(p.verify("p", &f.artifact)).contains("does not match its signature"));
    }

    #[test]
    fn malformed_and_oversized_signatures_are_errors_not_unsigned() {
        let f = signed(b"bytes");
        let p = policy(&[key(&f.signer)], Some(Unsigned::Allow));
        let sig = signature_path(&f.artifact);
        std::fs::write(&sig, "untrusted comment: x\nnot base64\n").unwrap();
        assert!(reason(p.verify("p", &f.artifact)).contains("minisig"));
        std::fs::write(&sig, "a".repeat(MAX_SIGNATURE_BYTES as usize + 1)).unwrap();
        assert!(reason(p.verify("p", &f.artifact)).contains("larger than 4096 bytes"));
    }

    #[test]
    fn a_missing_signature_follows_the_unsigned_mode() {
        let f = signed(b"bytes");
        std::fs::remove_file(signature_path(&f.artifact)).unwrap();
        let k = [key(&f.signer)];
        let deny = reason(policy(&k, Some(Unsigned::Deny)).verify("p", &f.artifact));
        assert_eq!(deny, "no signature, and the trust policy of plugin `p` refuses unsigned artifacts");
        assert!(reason(policy(&k, None).verify("p", &f.artifact)).contains("refuses"), "keys default to deny");
        for mode in [Unsigned::Warn, Unsigned::Allow] {
            assert_eq!(policy(&k, Some(mode)).verify("p", &f.artifact).unwrap(), Verdict::Unsigned);
        }
    }

    #[test]
    fn without_keys_a_signature_is_ignored_and_nothing_is_read() {
        let f = signed(b"bytes");
        for p in [TrustPolicy::default(), policy(&[], Some(Unsigned::Warn))] {
            assert_eq!(p.verify("p", &f.artifact).unwrap(), Verdict::Unsigned);
        }
        // The default policy touches no file, so it costs nothing on the load path.
        let missing = f.artifact.with_file_name("missing.so");
        assert_eq!(TrustPolicy::default().verify("p", &missing).unwrap(), Verdict::Unsigned);
        let err = reason(policy(&[], Some(Unsigned::Deny)).verify("p", &f.artifact));
        assert!(err.starts_with("no trusted keys"), "{err}");
    }

    #[test]
    fn a_missing_artifact_with_a_signature_is_an_io_error() {
        let f = signed(b"bytes");
        let moved = f.artifact.with_file_name("gone.so");
        std::fs::rename(signature_path(&f.artifact), signature_path(&moved)).unwrap();
        let r = policy(&[key(&f.signer)], None).verify("p", &moved);
        assert!(matches!(r, Err(PluginError::Io { path, .. }) if path == moved));
    }

    #[test]
    fn policies_parse_validate_and_display() {
        let k = key(&KeyPair::generate_unencrypted_keypair().unwrap());
        let p: TrustPolicy = toml::from_str(&format!("keys = [\"{k}\"]\nunsigned = \"warn\"")).unwrap();
        assert_eq!((p.unsigned_mode(), p.validate()), (Unsigned::Warn, Ok(())));
        assert_eq!(p.to_string(), format!("unsigned warned about, keys {}…", &k[..12]));
        assert_eq!(TrustPolicy::default().to_string(), "unsigned allowed, no keys");

        assert!(toml::from_str::<TrustPolicy>("unsigned = \"maybe\"").is_err());
        assert!(toml::from_str::<TrustPolicy>("key = []").unwrap_err().to_string().contains("unknown field"));
        let bad = policy(&["RWQnot-a-key".into()], None).validate().unwrap_err();
        assert!(bad.contains("`RWQnot-a-key` is not a minisign public key"), "{bad}");
        assert!(policy(&[], Some(Unsigned::Deny)).validate().unwrap_err().contains("needs at least one key"));
    }
}
