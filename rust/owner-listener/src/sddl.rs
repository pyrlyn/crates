//! SDDL for an owner-only named-pipe DACL.
//!
//! The string is assembled here, on every platform, so a sid cannot change
//! the descriptor's shape and the check does not need Windows.

/// Longest `S-1-…` string this crate will place in an SDDL blob.
const MAX_SID_LEN: usize = 184;

/// `sid` is not a Windows SID this crate will embed in an SDDL string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidError;

impl std::fmt::Display for SidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not a Windows SID")
    }
}

impl std::error::Error for SidError {}

/// Protected DACL with one allow-all ACE for `sid` and no other entry.
///
/// `D:P(A;;GA;;;<sid>)` grants the user every right on the pipe, including
/// creating further instances, and grants nothing to Everyone, Administrators,
/// or the network.
pub fn user_only_dacl(sid: &str) -> Result<String, SidError> {
    if !is_sid(sid) {
        return Err(SidError);
    }
    Ok(format!("D:P(A;;GA;;;{sid})"))
}

/// Whether `sid` is `S-` plus a revision and at least one more numeric
/// component, with no extra characters.
pub fn is_sid(sid: &str) -> bool {
    if sid.len() > MAX_SID_LEN {
        return false;
    }
    let Some(rest) = sid.strip_prefix("S-") else {
        return false;
    };
    let mut parts = rest.split('-');
    let Some(revision) = parts.next() else {
        return false;
    };
    if !is_digits(revision) {
        return false;
    }
    let mut more = 0u32;
    for part in parts {
        if !is_digits(part) {
            return false;
        }
        more += 1;
    }
    more >= 1
}

fn is_digits(part: &str) -> bool {
    !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_sid_becomes_a_single_allow_ace() {
        let sid = "S-1-5-21-100-200-300-1001";
        assert_eq!(
            user_only_dacl(sid).unwrap(),
            "D:P(A;;GA;;;S-1-5-21-100-200-300-1001)"
        );
    }

    #[test]
    fn a_sid_cannot_inject_another_ace() {
        for bad in [
            "",
            "S-1",
            "s-1-5-18",
            "S-1-5-21-1);D:(A;;GA;;;WD",
            "S-1-5-18 ",
            "S-1-5-18\n",
            "S--1-5",
            "S-1-5-",
            "not-a-sid",
        ] {
            assert!(user_only_dacl(bad).is_err(), "{bad}");
        }
    }
}
