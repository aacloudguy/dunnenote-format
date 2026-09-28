//! The version gate: what this library may do with a notebook at a given schema version.

use crate::error::{Error, Result};
use crate::schema::SCHEMA_VERSION;

/// How far ahead of [`SCHEMA_VERSION`] a notebook may be and still be read. DunneNote uses the
/// same tolerance: a notebook one version ahead opens read-only.
pub const MAX_FORWARD_DRIFT: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compat {
    /// Exactly the supported schema: readable (and, once writing ships, writable).
    Exact,
    /// One version ahead: readable, never writable. Newer columns are ignored.
    NewerReadOnly { found: u32 },
}

impl Compat {
    pub fn classify(found: u32) -> Result<Self> {
        if found == SCHEMA_VERSION {
            Ok(Compat::Exact)
        } else if found < SCHEMA_VERSION {
            Err(Error::SchemaTooOld {
                found,
                supported: SCHEMA_VERSION,
            })
        } else if found <= SCHEMA_VERSION + MAX_FORWARD_DRIFT {
            Ok(Compat::NewerReadOnly { found })
        } else {
            Err(Error::SchemaTooNew {
                found,
                supported: SCHEMA_VERSION,
            })
        }
    }

    pub fn writable(self) -> bool {
        matches!(self, Compat::Exact)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_all_four_bands() {
        assert_eq!(Compat::classify(18).unwrap(), Compat::Exact);
        assert_eq!(
            Compat::classify(19).unwrap(),
            Compat::NewerReadOnly { found: 19 }
        );
        assert!(matches!(
            Compat::classify(20),
            Err(Error::SchemaTooNew { found: 20, .. })
        ));
        assert!(matches!(
            Compat::classify(17),
            Err(Error::SchemaTooOld { found: 17, .. })
        ));
        assert!(Compat::Exact.writable());
        assert!(!Compat::NewerReadOnly { found: 19 }.writable());
    }
}
