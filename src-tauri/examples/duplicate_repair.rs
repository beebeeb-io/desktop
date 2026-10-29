//! Task 2274: the host of the duplicates repair tool (`src/duplicate_repair`). Built only with the feature:
//! `cargo run --example duplicate_repair --features duplicate-repair -- <subcommand>`. It installs no tracing
//! subscriber, so nothing but the tool's own counts reaches the screen (spec I5).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match beebeeb_desktop_lib::duplicate_repair::run(args) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}
