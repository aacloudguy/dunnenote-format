//! The language-neutral test vectors in `vectors/`, published with `SPEC.md` for implementers in
//! other languages, must stay exactly what this library computes.

use dunnenote_format::{fold::fold, position};
use serde_json::Value;

fn load(name: &str) -> Vec<Value> {
    let path = format!("{}/../../vectors/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

#[test]
fn positions_match_the_published_vectors() {
    let vectors = load("positions.json");
    assert!(vectors.len() > 400);
    for v in &vectors {
        let args: Vec<&str> = v["args"]
            .as_array()
            .map_or(Vec::new(), |a| a.iter().map(s).collect());
        let got = match s(&v["op"]) {
            "first" => Some(position::first()),
            "after" => position::after(args[0]).ok(),
            "before" => position::before(args[0]).ok(),
            "between" => position::between(args[0], args[1]).ok(),
            op => panic!("unknown op {op}"),
        };
        assert_eq!(got.as_deref(), v["result"].as_str(), "{v}");
    }
}

#[test]
fn fold_matches_the_published_vectors() {
    let vectors = load("fold.json");
    assert!(vectors.len() > 40);
    for v in &vectors {
        assert_eq!(fold(s(&v["input"])), s(&v["fold"]), "{v}");
    }
}
