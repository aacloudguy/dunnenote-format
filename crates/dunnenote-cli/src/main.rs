//! `dnfmt` — inspect, verify, export and edit DunneNote notebooks.

fn main() {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("--version") | Some("-V") => println!(
            "dnfmt {} (DunneNote Format {}, schema {})",
            env!("CARGO_PKG_VERSION"),
            dunnenote_format::FORMAT_VERSION,
            dunnenote_format::SCHEMA_VERSION
        ),
        _ => {
            eprintln!("dnfmt: pre-release. Commands arrive in later milestones.");
            eprintln!("usage: dnfmt --version");
            std::process::exit(2);
        }
    }
}
