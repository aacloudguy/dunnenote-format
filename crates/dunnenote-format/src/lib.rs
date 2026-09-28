//! Read and write DunneNote notebooks (`.dunnenote` bundles): the open DunneNote Format.
//!
//! A notebook is a directory holding `format.json`, a SQLite database `notebook.db`, a
//! content-addressed blob store and a few auxiliary folders. See `SPEC.md` in this repository.
//!
//! ```no_run
//! use dunnenote_format::Notebook;
//!
//! let nb = Notebook::open("Research.dunnenote")?;
//! for page in nb.pages()? {
//!     println!("{} ({} canvases)", page.name, nb.canvases(&page.id)?.len());
//! }
//! # Ok::<(), dunnenote_format::Error>(())
//! ```
//!
//! **Status:** pre-release. Reading and exporting (JSON, Markdown, CSV) are available; writing
//! lands in a later release.

pub mod error;
pub mod export;
pub mod gate;
pub mod manifest;
pub mod model;
pub mod notebook;
pub mod schema;
pub mod text;
pub mod verify;

pub use error::{Error, Result};
pub use gate::Compat;
pub use manifest::Manifest;
pub use model::*;
pub use notebook::Notebook;
pub use schema::{create_schema, FORMAT_VERSION, SCHEMA_SQL, SCHEMA_VERSION};
pub use verify::{verify, Finding, Report, Severity, VerifyLevel};
