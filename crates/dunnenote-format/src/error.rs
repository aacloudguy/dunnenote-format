use std::path::PathBuf;

/// Everything that can go wrong opening or reading a notebook.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} is not a DunneNote notebook: {1}")]
    NotANotebook(PathBuf, &'static str),

    #[error("format.json is invalid: {0}")]
    ManifestInvalid(String),

    #[error(
        "this notebook uses DunneNote Format {found}, which is newer than this library supports \
         ({supported}); update dunnenote-format"
    )]
    FormatTooNew { found: String, supported: String },

    #[error(
        "this notebook is at schema {found}, older than {supported}; open it once in DunneNote \
         to upgrade it, then try again"
    )]
    SchemaTooOld { found: u32, supported: u32 },

    #[error(
        "this notebook is at schema {found}, newer than this library supports ({supported}); \
         update dunnenote-format"
    )]
    SchemaTooNew { found: u32, supported: u32 },

    #[error("no {kind} with id {id}")]
    NotFound { kind: &'static str, id: String },

    #[error("stored data is malformed: {0}")]
    Malformed(String),

    #[error("blob {hash} does not match its content (hash {actual})")]
    BlobCorrupt { hash: String, actual: String },

    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
