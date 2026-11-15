//! The media helper's process: one file, read from the app over standard input and output, then gone.
//!
//! The scaffold's loop answers every `Open` with `Error(NoDecoder)` (nothing decodes yet) and exits on `Close` or
//! when the app's pipe ends; slice M3 hands the loop to [`beebeeb_media_helper::session`]. It writes nothing but
//! protocol messages to standard output, and nothing at all to standard error (the sandbox drops it).

#![deny(unsafe_code)]

use std::io;

use beebeeb_media_protocol::{ErrorCode, FromHelper, ToHelper, read_to_helper, write_msg};

fn main() {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    loop {
        match read_to_helper(&mut input) {
            Ok(ToHelper::Open { .. }) => {
                if write_msg(&mut output, &FromHelper::Error(ErrorCode::NoDecoder)).is_err() {
                    return;
                }
            }
            // `Close`, the app gone, or a message this protocol does not read: the helper ends.
            Ok(ToHelper::Close) | Err(_) => return,
            Ok(_) => {}
        }
    }
}
