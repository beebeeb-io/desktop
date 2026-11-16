//! The media helper's process: one file, read from the app over standard input and output, then gone.
//!
//! It writes nothing but protocol messages to standard output, and nothing at all to standard error (the sandbox
//! drops it). The whole loop is [`beebeeb_media_helper::session::run`].

#![deny(unsafe_code)]

fn main() {
    beebeeb_media_helper::session::run(std::io::stdin(), std::io::stdout());
}
