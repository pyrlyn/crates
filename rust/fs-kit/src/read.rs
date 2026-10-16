//! Reading where a missing file is an empty one: a config or state file nobody wrote yet.

use std::io::{self, ErrorKind};
use std::path::Path;

/// The text of `path`, or `""` when there is no file. Any other error names the path.
pub fn text_or_empty(path: &Path) -> io::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(crate::at(path, e)),
    }
}

/// The bytes of `path`, or `None` when there is no file. Any other error names the path.
pub fn bytes_if_exists(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(crate::at(path, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_reads_as_empty_and_a_directory_is_an_error() {
        let dir = std::env::temp_dir().join(format!("fs-kit-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(text_or_empty(&dir.join("nope")).unwrap(), "");
        assert_eq!(bytes_if_exists(&dir.join("nope")).unwrap(), None);
        std::fs::write(dir.join("a"), "x").unwrap();
        assert_eq!(text_or_empty(&dir.join("a")).unwrap(), "x");
        assert_eq!(
            bytes_if_exists(&dir.join("a")).unwrap(),
            Some(b"x".to_vec())
        );
        let err = text_or_empty(&dir).unwrap_err();
        assert!(
            err.to_string().contains(&dir.display().to_string()),
            "{err}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
