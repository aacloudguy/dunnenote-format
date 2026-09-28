//! `format.json` — the notebook manifest.
//!
//! The manifest is written once, when a notebook is created. Its `format_version` therefore
//! records the version the notebook was *created* at; the database's `PRAGMA user_version` is
//! what says which schema the notebook is at now (see [`crate::gate`]).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The value of `format` in every valid manifest.
pub const FORMAT_DISCRIMINATOR: &str = "dunnenote";

/// Highest `format_version` major version this library accepts.
pub const SUPPORTED_MAJOR: u64 = 0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Always `"dunnenote"`.
    pub format: String,
    /// Semantic version, `"0.<schema>.0"` at creation.
    pub format_version: String,
    /// The notebook's permanent identity (a UUID). Survives moves and renames.
    pub notebook_id: String,
    /// Unix seconds at creation.
    pub created_at: i64,
    /// The software that created the notebook, e.g. `"dunnenote-svc-file/0.9.0"`.
    pub created_by: String,
}

impl Manifest {
    pub fn read(root: &Path) -> Result<Self> {
        let bytes = std::fs::read(root.join("format.json"))?;
        let manifest: Manifest = serde_json::from_slice(&bytes)
            .map_err(|e| Error::ManifestInvalid(format!("not valid manifest JSON: {e}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != FORMAT_DISCRIMINATOR {
            return Err(Error::ManifestInvalid(format!(
                "format is {:?}, expected {FORMAT_DISCRIMINATOR:?}",
                self.format
            )));
        }
        let major = self
            .format_version
            .split('.')
            .next()
            .and_then(|m| m.parse::<u64>().ok())
            .filter(|_| self.format_version.split('.').count() >= 3)
            .ok_or_else(|| {
                Error::ManifestInvalid(format!(
                    "format_version {:?} is not a version number",
                    self.format_version
                ))
            })?;
        if major > SUPPORTED_MAJOR {
            return Err(Error::FormatTooNew {
                found: self.format_version.clone(),
                supported: format!("{SUPPORTED_MAJOR}.x"),
            });
        }
        if self.notebook_id.len() != 36 {
            return Err(Error::ManifestInvalid("notebook_id is not a UUID".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(format: &str, version: &str) -> Manifest {
        Manifest {
            format: format.into(),
            format_version: version.into(),
            notebook_id: "01920000-0000-7000-8000-000000000001".into(),
            created_at: 1_700_000_000,
            created_by: "test".into(),
        }
    }

    #[test]
    fn accepts_any_0x_version() {
        assert!(manifest("dunnenote", "0.18.0").validate().is_ok());
        assert!(manifest("dunnenote", "0.1.0").validate().is_ok());
    }

    #[test]
    fn rejects_wrong_discriminator_including_the_retired_name() {
        assert!(manifest("calnote", "0.18.0").validate().is_err());
    }

    #[test]
    fn rejects_major_1_and_garbage() {
        assert!(matches!(
            manifest("dunnenote", "1.0.0").validate(),
            Err(Error::FormatTooNew { .. })
        ));
        assert!(manifest("dunnenote", "eighteen").validate().is_err());
    }
}
