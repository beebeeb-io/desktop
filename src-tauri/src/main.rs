#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! The thin binary: `main` hands straight to `beebeeb_desktop_lib::run` so the app is one library
//! plus an entry point. Must not grow app logic of its own.

fn main() {
    beebeeb_desktop_lib::run()
}
