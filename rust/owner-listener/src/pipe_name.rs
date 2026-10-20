//! Named-pipe path checks.
//!
//! The Windows listener is the only production caller. The checks are pure
//! so they run in CI on every platform.

use std::io::{self, ErrorKind};

/// `\\.\pipe\<name>` with a single path segment and no control characters.
pub fn validate_pipe_name(name: &str) -> io::Result<()> {
    let Some(rest) = name.strip_prefix(r"\\.\pipe\") else {
        return Err(invalid("pipe name must start with \\\\.\\pipe\\"));
    };
    if rest.is_empty() || rest.contains(['\\', '/']) || rest.chars().any(char::is_control) {
        return Err(invalid(
            "pipe name must be one segment without control characters",
        ));
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_pipe_name_is_accepted() {
        validate_pipe_name(r"\\.\pipe\cofre-agent").unwrap();
    }

    #[test]
    fn a_name_that_escapes_the_pipe_root_is_refused() {
        for bad in [
            "",
            "cofre-agent",
            r"\\.\pipe\",
            r"\\.\pipe\cofre\agent",
            r"\\.\pipe\cofre/agent",
            "\\\\.\\pipe\\cofre\nagent",
            r"\\server\pipe\cofre",
        ] {
            assert!(validate_pipe_name(bad).is_err(), "{bad}");
        }
    }
}
