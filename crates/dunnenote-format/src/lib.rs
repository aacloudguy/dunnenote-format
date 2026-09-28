//! Read and write DunneNote notebooks (`.dunnenote` bundles): the open DunneNote Format.
//!
//! A notebook is a directory holding `format.json`, a SQLite database `notebook.db`, a
//! content-addressed blob store and a few auxiliary folders. See `SPEC.md` in this repository.
//!
//! **Status:** pre-release. This crate currently exposes the canonical schema; the reader and
//! writer land in later milestones.

pub mod schema;

pub use schema::{create_schema, FORMAT_VERSION, SCHEMA_SQL, SCHEMA_VERSION};
