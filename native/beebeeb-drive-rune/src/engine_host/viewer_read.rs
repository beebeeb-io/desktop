//! Where an open file's bytes come from: the copy on this PC, when the local state says it is here, it really is,
//! and the engine says it is not behind the version Beebeeb has (`LocalCopy::Behind`); else the download.

use super::*;

/// What the local state said of the file's copy on this PC when the file was opened.
#[derive(Clone, Copy)]
pub(super) struct Recorded {
    /// Its bytes are here (`OpenFileFacts::on_this_pc`).
    pub(super) here: bool,
    /// The copy here is not settled with Beebeeb's version (`OpenFileFacts::unsettled_here`).
    pub(super) unsettled: bool,
}

/// The file's bytes: the copy on this PC when the local state says it is here and it really is (no network at
/// all), else the download. The answer says which, and the session the bytes were read for. The buffer is the
/// engine's own, wiped when it is dropped. At most `max_bytes` are read (`IN_MEMORY_MAX` for the viewer's own kinds,
/// the Office contract's limit for an Office file).
///
/// The origin never says more than is known: a copy in sync by the local state is only that while sync runs
/// (with sync off nothing has compared it with Beebeeb since), and Beebeeb's version of a file whose copy here
/// is not settled with it may not be what is on this PC.
pub(super) fn read<E: ViewerEngine>(
    engine: &E,
    file_id: &str,
    recorded: Recorded,
    max_bytes: u64,
    progress: &(dyn Fn(u64, u64) + Send + Sync + 'static),
    cancel: &AtomicBool,
) -> Result<(SessionGeneration, zeroize::Zeroizing<Vec<u8>>, Origin), OpenFileError> {
    let mut behind = false;
    if recorded.here {
        match engine.local(file_id, max_bytes, cancel)? {
            LocalCopy::Opened(copy) => {
                let origin = match copy.copy {
                    OnThisPc::NotSentYet => Origin::ThisPc { unsent: true },
                    OnThisPc::InSync if engine.is_syncing() => Origin::ThisPc { unsent: false },
                    OnThisPc::InSync => Origin::ThisPcSyncOff,
                };
                return Ok((copy.session, copy.bytes, origin));
            }
            // Not wholly on this PC after all, and in sync: the download is the way.
            LocalCopy::NotHere => {}
            // Behind the version Beebeeb has now (the engine's rule): Beebeeb's, said as not this PC's copy.
            LocalCopy::Behind => behind = true,
        }
    }
    let opened = engine.fetch(file_id, max_bytes, progress, cancel)?;
    // A copy here with a change of its own not settled says the most: Beebeeb's version is not what is on this PC.
    // Only then is a copy behind it said as one this PC could not confirm (delta check of desktop#432, round 3).
    let origin = if recorded.unsettled {
        Origin::BeebeebNotThisPcs
    } else if behind {
        Origin::BeebeebUnconfirmedHere
    } else {
        Origin::Beebeeb
    };
    Ok((opened.session, opened.bytes, origin))
}
