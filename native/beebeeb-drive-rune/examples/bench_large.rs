//! Bench: the Files window on a 40,000-file account (`LargeAccount::GUUS` on the engine's fake server).
//!
//! NOT BUILT HERE YET. The bench has to build the app's real frames, so it declares every module of `src/main.rs` at
//! its own crate root, and that needs `src/engine_host.rs` and `src/log_file.rs` to be `engine_host/mod.rs` and
//! `log_file/mod.rs` (a `#[path]` file reads its children from its own folder). Five test files read those two paths
//! (`src/shell/mod.rs`, `src/engine_host/{tests,sync_tests,launcher_tests,launch_tests,files_downloads_tests}.rs`),
//! so the move is the lead's call. The finished, run example (all eight views, p50/p95 frame ms and RSS MB) is on
//! branch `native/dc-025-probe` (`examples/bench_large.rs`): take it with the two moves, then fix those paths.
//!
//! It also needs `--config profile.release.debug-assertions=true`: the engine refuses `test-support` in a release
//! build without debug assertions (`crates/beebeeb-engine/src/lib.rs`), which a guard we do not loosen.

fn main() {
    eprintln!("bench_large: see the comment at the top of this file (branch native/dc-025-probe has the working bench)");
    std::process::exit(2);
}
