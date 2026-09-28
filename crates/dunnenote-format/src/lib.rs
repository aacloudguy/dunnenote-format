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
//! Writing goes through [`Notebook::create`] or [`Notebook::open_writable`] and
//! [`Notebook::write`], which follow the format's writer rules (see [`write`]):
//!
//! ```no_run
//! use dunnenote_format::{payload, At, Frame, Notebook, Settings};
//!
//! let mut nb = Notebook::create("Trip.dunnenote", None)?;
//! let root = nb.notebook_node()?.id;
//! nb.write(|w| {
//!     let section = w.add_section(&root, "Plans", At::End)?;
//!     let page = w.add_page(&section, "Day 1", At::End)?;
//!     let doc = payload::rich_text_from_markdown("# Day 1\n\nTrain at **9:10**.");
//!     w.add_rich_text(&page, Frame::PAGE_TEXT, Some(&doc), &Settings::new())?;
//!     Ok(())
//! })?;
//! # Ok::<(), dunnenote_format::Error>(())
//! ```
//!
//! **Status:** pre-release. Reading, exporting (JSON, Markdown, CSV) and writing pages, sections,
//! rich text, sketches, pictures and canvas groups are available; tables, forms, calendars, tags,
//! archiving and templates land in a later release.

pub mod error;
pub mod export;
pub mod gate;
pub mod manifest;
pub mod model;
pub mod notebook;
pub mod payload;
pub mod position;
pub mod schema;
pub mod text;
pub mod verify;
pub mod write;

pub use error::{Error, Result};
pub use gate::Compat;
pub use manifest::Manifest;
pub use model::*;
pub use notebook::Notebook;
pub use schema::{create_schema, FORMAT_VERSION, SCHEMA_SQL, SCHEMA_VERSION};
pub use verify::{verify, Finding, Report, Severity, VerifyLevel};
pub use write::{At, Frame, Writer};
