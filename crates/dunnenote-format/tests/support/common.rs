//! Helpers shared by the write tests.
#![allow(dead_code)]

use std::path::Path;

use dunnenote_format::{verify, At, Notebook, VerifyLevel};
use rusqlite::Connection;
use tempfile::TempDir;

pub fn raw(root: &Path) -> Connection {
    Connection::open(root.join("notebook.db")).unwrap()
}

pub fn count(root: &Path, sql: &str) -> i64 {
    raw(root).query_row(sql, [], |r| r.get(0)).unwrap()
}

pub fn new_notebook(dir: &TempDir, name: &str) -> (Notebook, String) {
    let nb = Notebook::create(dir.path().join(format!("{name}.dunnenote")), None).unwrap();
    let root = nb.notebook_node().unwrap().id;
    (nb, root)
}

pub fn page_in(nb: &mut Notebook, root: &str) -> String {
    nb.write(|w| {
        let s = w.add_section(root, "S", At::End)?;
        w.add_page(&s, "P", At::End)
    })
    .unwrap()
}

pub fn assert_clean(root: &Path) {
    let nb = Notebook::open(root).unwrap();
    let report = verify(&nb, VerifyLevel::Full).unwrap();
    let problems: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check != "search_index")
        .collect();
    assert!(problems.is_empty(), "{}: {problems:?}", root.display());
}
