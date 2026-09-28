//! Emit notebooks written by this library, for DunneNote's own conformance tests.
//!
//! ```text
//! cargo run -p dunnenote-format --example emit_written -- <out-dir>
//! ```
//!
//! `<out-dir>` must not exist. It receives:
//!
//! - `written-*.dunnenote`: the notebooks built in `tests/support/written.rs`;
//! - `expected.json`: what this library reads in each one, in the shape of
//!   `fixtures/expected.json`. DunneNote's round-trip test opens each notebook through the app's
//!   own open path and requires the app to read exactly this;
//! - `payloads/`: every rich text document and sketch this library stored, byte for byte, one
//!   file each (`<notebook>--<canvas id>.rich_text.json` / `.sketch.json`). Content that DunneNote
//!   wrote (the untouched canvases of `written-into-golden`) is left out. DunneNote's payload test
//!   parses them with the editor's own schema and stroke reader.

use std::path::PathBuf;

use dunnenote_format::{CanvasKind, Notebook};
use serde_json::{json, Map};

#[path = "../tests/support/observe.rs"]
mod observe;
#[path = "../tests/support/written.rs"]
mod written;

fn main() {
    if let Err(e) = run() {
        eprintln!("emit_written: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: emit_written <out-dir>")?;
    if out.exists() {
        return Err(format!("{} already exists", out.display()).into());
    }
    std::fs::create_dir_all(out.join("payloads"))?;

    let golden = Notebook::open(written::fixtures().join("every-kind.dunnenote"))?;
    let stored_in = |nb: &Notebook, table: &str, id: &str| -> Option<String> {
        nb.connection()
            .query_row(
                &format!("SELECT data FROM {table} WHERE instance_id = ?1"),
                [id],
                |r| r.get(0),
            )
            .ok()
    };
    let mut notebooks = Map::new();
    for (name, root) in written::build_all(&out)? {
        let nb = Notebook::open(&root)?;
        notebooks.insert(name.into(), observe::observe(&nb));
        for page in nb.pages()? {
            for c in nb.canvases(&page.id)? {
                let (table, suffix) = match c.kind {
                    CanvasKind::RichText => ("rich_text_instances", "rich_text"),
                    CanvasKind::Sketch | CanvasKind::Picture => ("sketch_instances", "sketch"),
                    _ => continue,
                };
                let stored = stored_in(&nb, table, &c.id);
                if stored.is_some() && stored == stored_in(&golden, table, &c.id) {
                    continue; // written by DunneNote, not by this library
                }
                if let Some(data) = stored {
                    std::fs::write(
                        out.join("payloads")
                            .join(format!("{name}--{}.{suffix}.json", c.id)),
                        data,
                    )?;
                }
            }
        }
        println!("wrote {name}.dunnenote");
    }
    std::fs::write(
        out.join("expected.json"),
        serde_json::to_string_pretty(&json!({ "notebooks": notebooks }))? + "\n",
    )?;
    Ok(())
}
