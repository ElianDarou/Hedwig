use serde::{Deserialize, Serialize};
use std::{fmt, path::Path};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CloudPath(String);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathError {
    #[error("der Pfad muss relativ sein")]
    Absolute,
    #[error("der Pfad enthält ein unzulässiges Segment")]
    InvalidSegment,
    #[error("der Pfad ist leer")]
    Empty,
    #[error("der Pfad enthält ein Nullbyte")]
    NullByte,
}

impl CloudPath {
    pub fn parse(value: impl Into<String>) -> Result<Self, PathError> {
        let value = value.into().replace('\\', "/");
        if value.contains('\0') {
            return Err(PathError::NullByte);
        }
        if value.is_empty() {
            return Err(PathError::Empty);
        }
        if Path::new(&value).is_absolute() || value.starts_with('/') {
            return Err(PathError::Absolute);
        }
        let parts: Vec<&str> = value.split('/').collect();
        if parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == "..")
        {
            return Err(PathError::InvalidSegment);
        }
        Ok(Self(parts.join("/")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().expect("validated non-empty path")
    }
}

impl fmt::Display for CloudPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_windows_separators() {
        assert_eq!(
            CloudPath::parse("docs\\plan.md").unwrap().as_str(),
            "docs/plan.md"
        );
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        assert!(matches!(
            CloudPath::parse("../secret"),
            Err(PathError::InvalidSegment)
        ));
        assert!(matches!(
            CloudPath::parse("/etc/passwd"),
            Err(PathError::Absolute)
        ));
    }
}
