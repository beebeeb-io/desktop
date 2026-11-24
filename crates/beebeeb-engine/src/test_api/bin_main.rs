//! `bb-fake-server`: the engine's fixture account on loopback, so a TEST build started with `--api-base
//! http://127.0.0.1:<port>` can sign in on a computer with no server (task 2475). A development binary: built only
//! with `--features fake-server`, never in a release profile, never part of an app. Everything it does is
//! `beebeeb_engine::test_api::server` (tested there and in `tests/fake_server.rs`).

fn main() -> std::process::ExitCode {
    beebeeb_engine::test_api::server::run(std::env::args().skip(1))
}
