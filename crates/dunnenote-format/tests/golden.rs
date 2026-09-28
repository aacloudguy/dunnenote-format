//! Conformance against notebooks written by DunneNote itself.
//!
//! `fixtures/*.dunnenote` were produced by DunneNote's own code (its golden-notebook emitter
//! drives the same functions the app's interface calls), and `fixtures/expected.json` records
//! what DunneNote's own read paths report about each one. This library must report the same
//! thing, field for field. See `fixtures/README.md`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use dunnenote_format::{export, verify, CanvasKind, Notebook, VerifyLevel};
use serde_json::{Map, Value};
use tempfile::TempDir;

#[path = "support/observe.rs"]
mod support;
use support::{first_difference, observe};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn expected() -> Map<String, Value> {
    let text = std::fs::read_to_string(fixtures().join("expected.json")).expect("expected.json");
    let v: Value = serde_json::from_str(&text).expect("expected.json parses");
    v["notebooks"].as_object().expect("notebooks").clone()
}

fn open(name: &str) -> Notebook {
    Notebook::open(fixtures().join(format!("{name}.dunnenote")))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// The golden notebooks on disk, by name.
fn on_disk() -> BTreeSet<String> {
    std::fs::read_dir(fixtures())
        .expect("fixtures dir")
        .filter_map(|e| {
            let name = e.ok()?.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".dunnenote").map(str::to_string)
        })
        .collect()
}

// ---- the tests ----------------------------------------------------------------------------------

#[test]
fn every_golden_notebook_has_an_expectation() {
    let want: BTreeSet<String> = expected().keys().cloned().collect();
    assert_eq!(
        on_disk(),
        want,
        "fixtures on disk and expected.json disagree"
    );
    assert!(want.len() >= 6, "the golden set shrank: {want:?}");
}

#[test]
fn the_library_reads_each_golden_notebook_as_dunnenote_does() {
    for (name, want) in expected() {
        let got = observe(&open(&name));
        if let Some(diff) = first_difference(&name, &got, &want) {
            panic!("{diff}");
        }
    }
}

#[test]
fn golden_notebooks_cover_every_canvas_kind_and_role() {
    let all = expected();
    let canvases: Vec<&Value> = all
        .values()
        .flat_map(|nb| nb["pages"].as_object().unwrap().values())
        .flat_map(|p| p["canvases"].as_array().unwrap())
        .collect();
    for kind in CanvasKind::ALL {
        assert!(
            canvases.iter().any(|c| c["kind"] == kind.as_str()),
            "no golden canvas of kind {}",
            kind.as_str()
        );
    }
    let has = |key: &str| canvases.iter().any(|c| c["settings"].get(key).is_some());
    for key in [
        "caption",
        "formField",
        "formTarget",
        "hidden",
        "templateKeepContent",
        "placeholder",
    ] {
        assert!(has(key), "no golden canvas carries settings.{key}");
    }
    assert!(canvases.iter().any(|c| c["lifecycle"] == "archived"));
    assert!(canvases
        .iter()
        .any(|c| c["kind"] == "picture" && !c["content"]["strokes"].is_null()));
    assert!(canvases.iter().any(|c| !c["group_id"].is_null()));
    let tree: Vec<&Value> = all
        .values()
        .flat_map(|nb| nb["tree"].as_array().unwrap())
        .collect();
    assert!(tree.iter().any(|n| n["is_template"] == true));
    assert!(tree.iter().any(|n| n["is_archived"] == true));
    let kinds: BTreeSet<&str> = all
        .values()
        .flat_map(|nb| nb["item_tags"].as_array().unwrap())
        .map(|t| t["source_kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        BTreeSet::from(["dataset", "dataset_row", "instance", "node"])
    );
}

#[test]
fn golden_notebooks_verify_clean() {
    for name in on_disk() {
        let report = verify(&open(&name), VerifyLevel::Full).unwrap();
        assert!(report.is_clean(), "{name}: {:?}", report.findings);
    }
}

#[test]
fn json_export_carries_everything_the_app_reads() {
    for (name, want) in expected() {
        let doc = export::to_json(&open(&name)).unwrap();
        assert_eq!(doc["export_version"], export::JSON_EXPORT_VERSION);
        let text = doc.to_string();
        // Every node and canvas id DunneNote reports appears in the export.
        for node in want["tree"].as_array().unwrap() {
            assert!(
                text.contains(node["id"].as_str().unwrap()),
                "{name}: node missing"
            );
        }
        for page in want["pages"].as_object().unwrap().values() {
            for c in page["canvases"].as_array().unwrap() {
                assert!(
                    text.contains(c["id"].as_str().unwrap()),
                    "{name}: canvas missing"
                );
            }
        }
        // And it round-trips through serde as a document, not just a string.
        let back: Value = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(back, doc);
    }
}

fn read_tree(dir: &Path) -> String {
    let mut out = String::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(s) = std::fs::read_to_string(&p) {
                out.push_str(&s);
            }
        }
    }
    out
}

#[test]
fn markdown_and_csv_exports_carry_the_visible_text() {
    let tmp = TempDir::new().unwrap();
    let nb = open("every-kind");
    let md = tmp.path().join("md");
    let summary = export::to_markdown(&nb, &md, export::Options::default()).unwrap();
    assert_eq!(summary.pages, 3);
    let text = read_tree(&md);
    for needle in [
        "## Project overview",
        "**one canvas of every kind**",
        "[web link](<https://example.com/>)",
        "| Draft the outline | Ada | 3 | true |",
        "| R3C1 | R3C2 | R3C3 |",
        "**Weekly stand-up**",
        "![Four coloured squares](<../assets/",
        "<path d=\"M30.00 40.00",
    ] {
        assert!(text.contains(needle), "markdown export lacks {needle:?}");
    }
    assert!(
        export::to_markdown(&nb, &md, export::Options::default()).is_err(),
        "a second export into the same folder must refuse, not overwrite"
    );

    let csv = tmp.path().join("csv");
    let summary = export::to_csv(&nb, &csv, export::Options::default()).unwrap();
    assert_eq!((summary.tables, summary.calendars), (2, 1));
    let text = read_tree(&csv);
    assert!(text.contains("Task,Owner,Hours,Done\r\nDraft the outline,Ada,3,true\r\n"));
    assert!(text.contains("Weekly stand-up,2026-01-05T15:00:00Z,2026-01-05T15:30:00Z,false,Room 4"));
}

#[test]
fn exports_skip_archived_and_hidden_content_unless_asked() {
    let tmp = TempDir::new().unwrap();
    let archive = open("archive");
    let plain = read_tree(&{
        let d = tmp.path().join("a");
        export::to_markdown(&archive, &d, export::Options::default()).unwrap();
        d
    });
    assert!(plain.contains("The paragraph that stays."));
    assert!(!plain.contains("An abandoned paragraph."));
    assert!(!plain.contains("Replaced by the second draft."));
    let all = read_tree(&{
        let d = tmp.path().join("b");
        export::to_markdown(
            &archive,
            &d,
            export::Options {
                include_archived: true,
            },
        )
        .unwrap();
        d
    });
    assert!(all.contains("An abandoned paragraph."));
    assert!(all.contains("Replaced by the second draft."));

    // The form's hidden signature carrier is not drawn; the answer token stays in the table.
    let forms = read_tree(&{
        let d = tmp.path().join("f");
        export::to_markdown(&open("forms"), &d, export::Options::default()).unwrap();
        d
    });
    assert_eq!(
        forms.matches("![Sketch]").count(),
        1,
        "hidden carrier was exported"
    );
    assert!(forms.contains("sketch:"));
}
