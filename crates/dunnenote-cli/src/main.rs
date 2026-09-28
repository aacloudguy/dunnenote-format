//! `dnfmt` — inspect, verify, export and edit DunneNote notebooks without DunneNote.

use std::process::ExitCode;

use std::path::Path;

use dunnenote_format::{
    export,
    text::{plain_text, utc_date, utc_datetime},
    verify, CanvasKind, Error, Node, NodeKind, Notebook, Severity, VerifyLevel, FORMAT_VERSION,
    SCHEMA_VERSION,
};
use serde_json::json;

mod edit;
use edit::{add_canvas, add_node, new_cmd, table_cmd};

const USAGE: &str = "\
dnfmt — read, check and edit DunneNote notebooks (.dunnenote)

USAGE:
    dnfmt inspect <notebook> [--json]      What the notebook is and what it holds
    dnfmt ls <notebook> [--json]           The section and page tree, with ids
    dnfmt cat <notebook> <id>              A page or canvas as text
    dnfmt verify <notebook> [--full] [--json] [--strict]
                                           Check for damage (exit 1 on errors;
                                           --strict also fails on warnings)
    dnfmt export --md <notebook> <folder> [--include-archived]
                                           Markdown files, pictures and sketches
    dnfmt export --csv <notebook> <folder> [--include-archived]
                                           One CSV file per table and calendar
    dnfmt export --json <notebook> [<file>]
                                           Everything, as one JSON document
                                           (to standard output without <file>)

    dnfmt new <notebook> [--name=<name>]    Create an empty notebook (folder ends in .dunnenote)
    dnfmt add-section <notebook> <parent-id|root> <name> [--first]
    dnfmt add-page <notebook> <parent-id|root> <name> [--first] [--no-text]
                                           A page, with DunneNote's empty text box
                                           unless --no-text
    dnfmt add-canvas <notebook> <page-id> rich-text [<file>|-]
                                           Text from Markdown (.md), a ProseMirror
                                           document (.json) or plain text; - is stdin
    dnfmt add-canvas <notebook> <page-id> sketch [<strokes.json>]
    dnfmt add-canvas <notebook> <page-id> picture <image> [--alt=<text>]
    dnfmt add-canvas <notebook> <page-id> table [<file.csv>|<file.json>]
                                           An empty Editable table, or a Data
                                           Table imported from a file
        canvas options: [--at=<x>,<y>] [--size=<width>,<height>]
    dnfmt table <notebook> <table-id> add-row [<column>=<value>…]
    dnfmt table <notebook> <table-id> set <row-id> <column> <value>
    dnfmt table <notebook> <table-id> add-column <name>
                                           Change an Editable table; a column is
                                           named or given by key (c0, c1, …)
    dnfmt --version

Reading commands open notebooks read-only and never change them. Editing
commands refuse a notebook that is open in DunneNote, print the new item's id,
and leave the search index for DunneNote to rebuild on its next open. Exports
never overwrite: the output folder must be new or empty, the output file new.";

pub struct Args {
    positional: Vec<String>,
    flags: Vec<String>,
}

impl Args {
    fn parse() -> Self {
        let (flags, positional) = std::env::args().skip(1).partition(|a| a.starts_with("--"));
        Self { positional, flags }
    }
    pub fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }
    /// The value of `--name=value`.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .find_map(|f| f.strip_prefix(name)?.strip_prefix('='))
    }
}

const BOOL_FLAGS: [&str; 10] = [
    "--json",
    "--full",
    "--strict",
    "--help",
    "--md",
    "--csv",
    "--include-archived",
    "--first",
    "--no-text",
    "--version",
];
const VALUE_FLAGS: [&str; 4] = ["--name", "--at", "--size", "--alt"];

fn main() -> ExitCode {
    let args = Args::parse();
    if args.flag("--version") {
        println!(
            "dnfmt {} (DunneNote Format {FORMAT_VERSION}, schema {SCHEMA_VERSION})",
            env!("CARGO_PKG_VERSION")
        );
        return ExitCode::SUCCESS;
    }
    if let Some(unknown) = args.flags.iter().find(|f| {
        let key = f.split('=').next().unwrap_or_default();
        !(BOOL_FLAGS.contains(&f.as_str()) || (f.contains('=') && VALUE_FLAGS.contains(&key)))
    }) {
        eprintln!("dnfmt: unknown option {unknown}\n\n{USAGE}");
        return ExitCode::from(2);
    }
    let pos: Vec<&str> = args.positional.iter().map(String::as_str).collect();
    let result = match pos.as_slice() {
        ["inspect", path] => inspect(path, args.flag("--json")),
        ["ls", path] => ls(path, args.flag("--json")),
        ["cat", path, id] => cat(path, id),
        ["export", path, rest @ ..] if rest.len() <= 1 => {
            export_cmd(&args, path, rest.first().copied())
        }
        ["new", path] => new_cmd(path, args.value("--name")),
        ["add-section", path, parent, name] => add_node(path, parent, name, false, &args),
        ["add-page", path, parent, name] => add_node(path, parent, name, true, &args),
        ["add-canvas", path, page, kind, rest @ ..] if rest.len() <= 1 => {
            add_canvas(path, page, kind, rest.first().copied(), &args)
        }
        ["table", path, table, op, rest @ ..] => table_cmd(path, table, op, rest),
        ["verify", path] => {
            return verify_cmd(
                path,
                args.flag("--full"),
                args.flag("--json"),
                args.flag("--strict"),
            )
        }
        _ => {
            let code = if args.flag("--help") || pos.is_empty() {
                0
            } else {
                2
            };
            eprintln!("{USAGE}");
            return ExitCode::from(code);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dnfmt: {e}");
            ExitCode::from(1)
        }
    }
}

fn inspect(path: &str, as_json: bool) -> Result<(), Error> {
    let nb = Notebook::open(path)?;
    let root = nb.notebook_node()?;
    let counts = nb.counts()?;
    let locked = nb.locked_by_another_process()?;
    if as_json {
        let out = json!({
            "name": root.name,
            "manifest": nb.manifest(),
            "schema_version": nb.schema_version(),
            "writable_by_this_version": nb.compat().writable(),
            "open_in_another_process": locked,
            "counts": counts,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return Ok(());
    }
    let m = nb.manifest();
    println!("{}", root.name);
    println!("  notebook id     {}", m.notebook_id);
    println!(
        "  schema          {} (created as format {})",
        nb.schema_version(),
        m.format_version
    );
    println!("  created by      {}", m.created_by);
    if !nb.compat().writable() {
        println!("  note            newer than this dnfmt; read-only");
    }
    if locked {
        println!("  note            open in DunneNote right now");
    }
    println!("  sections        {}", counts.sections);
    println!(
        "  pages           {} ({} archived, {} templates)",
        counts.pages, counts.archived_pages, counts.templates
    );
    println!(
        "  canvases        {} ({} archived)",
        counts.canvases, counts.archived_canvases
    );
    for (kind, n) in &counts.canvases_by_kind {
        let name = CanvasKind::parse(kind)
            .map(|k| k.display_name())
            .unwrap_or(kind);
        println!("    {name:<16}{n}");
    }
    println!("  tags            {}", counts.tags);
    println!(
        "  blobs           {} ({})",
        counts.blobs,
        human_bytes(counts.blob_bytes)
    );
    Ok(())
}

fn ls(path: &str, as_json: bool) -> Result<(), Error> {
    let nb = Notebook::open(path)?;
    let tree = nb.walk()?;
    if as_json {
        let items: Vec<_> = tree
            .iter()
            .map(|(depth, n)| json!({"depth": depth, "node": n}))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&items).unwrap_or_default()
        );
        return Ok(());
    }
    for (depth, node) in &tree {
        let mut label = node.name.clone();
        if node.kind == NodeKind::Page {
            let n = nb.canvases(&node.id)?.len();
            label.push_str(&format!("  ({n} canvas{})", if n == 1 { "" } else { "es" }));
        }
        if node.is_template {
            label.push_str("  [template]");
        }
        if node.is_archived {
            label.push_str("  [archived]");
        }
        let icon = match node.kind {
            NodeKind::Notebook => "▣",
            NodeKind::Group => "▸",
            NodeKind::Page => "·",
        };
        println!("{}{icon} {label}  {}", "  ".repeat(*depth), node.id);
    }
    Ok(())
}

fn cat(path: &str, id: &str) -> Result<(), Error> {
    let nb = Notebook::open(path)?;
    match nb.node(id) {
        Ok(node) => cat_page(&nb, &node),
        Err(Error::NotFound { .. }) => cat_canvas(&nb, id, true),
        Err(e) => Err(e),
    }
}

fn cat_page(nb: &Notebook, node: &Node) -> Result<(), Error> {
    if node.kind != NodeKind::Page {
        for child in nb.children(&node.id)? {
            println!("{}  {}", child.name, child.id);
        }
        return Ok(());
    }
    println!("# {}", node.name);
    for canvas in nb.canvases(&node.id)? {
        if canvas.is_hidden() || canvas.is_archived() {
            continue;
        }
        println!();
        cat_canvas(nb, &canvas.id, false)?;
    }
    Ok(())
}

fn cat_canvas(nb: &Notebook, id: &str, standalone: bool) -> Result<(), Error> {
    let canvas = nb.canvas(id)?;
    if !standalone {
        println!("[{} {}]", canvas.kind.display_name(), canvas.id);
    }
    match canvas.kind {
        CanvasKind::RichText => {
            if let Some(rt) = nb.rich_text(id)? {
                println!("{}", plain_text(&rt.doc));
            }
        }
        CanvasKind::Sketch => {
            let strokes = nb.sketch(id)?.map(|s| s.strokes().len()).unwrap_or(0);
            println!("(sketch: {strokes} strokes)");
        }
        CanvasKind::Picture => {
            let info = nb.blob_info(&canvas.source_hash)?;
            let alt = canvas
                .settings
                .get("alt")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let alt = if alt.is_empty() {
                String::new()
            } else {
                format!(", alt \"{alt}\"")
            };
            println!(
                "(picture: {}, sha256 {}{alt})",
                human_bytes(info.size_bytes),
                &canvas.source_hash[..12]
            );
            if let Some(markup) = nb.sketch(id)? {
                println!("(markup: {} strokes)", markup.strokes().len());
            }
        }
        CanvasKind::Database | CanvasKind::Spreadsheet => {
            if let Some(ds) = nb.dataset(id)? {
                let names: Vec<&str> = ds.columns.iter().map(|c| c.name.as_str()).collect();
                println!("{}", names.join("\t"));
                for row in &ds.rows {
                    println!("{}", ds.row_strings(row).join("\t"));
                }
            }
        }
        CanvasKind::Calendar => {
            for ev in nb.calendar_events(id)? {
                let when = if ev.all_day {
                    utc_date(ev.start_utc)
                } else {
                    format!(
                        "{} – {}",
                        utc_datetime(ev.start_utc),
                        utc_datetime(ev.end_utc)
                    )
                };
                let place = if ev.location.is_empty() {
                    String::new()
                } else {
                    format!(" @ {}", ev.location)
                };
                println!("{when}  {}{place}", ev.summary);
            }
        }
    }
    Ok(())
}

fn export_cmd(args: &Args, path: &str, out: Option<&str>) -> Result<(), Error> {
    let chosen: Vec<&str> = ["--md", "--csv", "--json"]
        .into_iter()
        .filter(|f| args.flag(f))
        .collect();
    let opts = export::Options {
        include_archived: args.flag("--include-archived"),
    };
    let usage = || {
        Error::Io(std::io::Error::other(format!(
            "choose one of --md, --csv or --json\n\n{USAGE}"
        )))
    };
    let nb = Notebook::open(path)?;
    match (chosen.as_slice(), out) {
        (["--json"], None) => {
            let doc = export::to_json(&nb)?;
            println!("{}", serde_json::to_string_pretty(&doc).unwrap_or_default());
            Ok(())
        }
        (["--json"], Some(file)) => {
            let doc = export::to_json(&nb)?;
            let text = serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n";
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(file)
                .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()))?;
            eprintln!("wrote {file}");
            Ok(())
        }
        ([kind @ ("--md" | "--csv")], Some(folder)) => {
            let summary = if *kind == "--md" {
                export::to_markdown(&nb, Path::new(folder), opts)?
            } else {
                export::to_csv(&nb, Path::new(folder), opts)?
            };
            let mut parts = vec![
                format!("{} pages", summary.pages),
                format!("{} tables", summary.tables),
            ];
            if summary.calendars > 0 {
                parts.push(format!("{} calendars", summary.calendars));
            }
            if summary.skipped_archived > 0 {
                parts.push(format!(
                    "{} archived items skipped",
                    summary.skipped_archived
                ));
            }
            eprintln!(
                "wrote {} files to {folder} ({})",
                summary.files.len(),
                parts.join(", ")
            );
            Ok(())
        }
        _ => Err(usage()),
    }
}

fn verify_cmd(path: &str, full: bool, as_json: bool, strict: bool) -> ExitCode {
    let nb = match Notebook::open(path) {
        Ok(nb) => nb,
        Err(e) => {
            eprintln!("dnfmt: {e}");
            return ExitCode::from(1);
        }
    };
    let level = if full {
        VerifyLevel::Full
    } else {
        VerifyLevel::Quick
    };
    let report = match verify(&nb, level) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("dnfmt: {e}");
            return ExitCode::from(1);
        }
    };
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
    } else {
        for f in &report.findings {
            let tag = match f.severity {
                Severity::Info => "info",
                Severity::Warning => "warning",
                Severity::Error => "ERROR",
            };
            println!("{tag:<8}{:<16}{}", f.check, f.detail);
        }
        let verdict = if report.has_errors() {
            "damaged"
        } else if report.is_clean() {
            "ok"
        } else {
            "ok, with warnings"
        };
        println!(
            "{verdict} — {} checks{}",
            report.checks_run.len(),
            if full { " (full)" } else { "" }
        );
    }
    if report.has_errors() || (strict && !report.is_clean()) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn human_bytes(n: i64) -> String {
    let n = n as f64;
    if n < 1024.0 {
        format!("{n} B")
    } else if n < 1024.0 * 1024.0 {
        format!("{:.1} KB", n / 1024.0)
    } else {
        format!("{:.1} MB", n / 1024.0 / 1024.0)
    }
}
