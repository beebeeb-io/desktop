//! The two-state flags the facade used to take as `bool`s, named.
//!
//! Each still accepts the old `bool` (an `impl Into<…>` parameter plus `From<bool>`):
//! the call sites are other lanes' files, so their argument is unchanged until those
//! lanes name it. Inside this module the value has a name, and `as_bool` is the one
//! place the stored 0/1 comes from.

/// Whether a pin write sets the state or clears it (the facade's `pinned: bool`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pinned {
    Pinned,
    Unpinned,
}

/// Whether a "not in sync" mark is to be made or is made (`set_mark_pending`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkPending {
    Pending,
    Settled,
}

/// Whether a staged payload is the completed upload or still in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadCompleted {
    Completed,
    InFlight,
}

/// Which mark a sign-out leaves for the File Provider domain (`clear_account_data`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignOutMarker {
    /// A removal is still owed: the domain is here and must be removed.
    RemovalOwed,
    /// The domain is gone already.
    DomainGone,
}

/// Whether this PC still owes the File Provider domain a removal (`set_finder_removal_owed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinderRemovalOwed {
    Owed,
    Settled,
}

/// Whether a snapshot node is mid-upload, so its version must not be trusted yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotNodeState {
    MidUpload,
    Settled,
}

macro_rules! two_state {
    ($name:ident, $set:ident, $clear:ident) => {
        impl $name {
            /// The stored flag: `true` is `$set`.
            pub fn as_bool(self) -> bool {
                matches!(self, Self::$set)
            }
        }

        impl From<bool> for $name {
            fn from(flag: bool) -> Self {
                if flag { Self::$set } else { Self::$clear }
            }
        }
    };
}

two_state!(Pinned, Pinned, Unpinned);
two_state!(MarkPending, Pending, Settled);
two_state!(PayloadCompleted, Completed, InFlight);
two_state!(SignOutMarker, RemovalOwed, DomainGone);
two_state!(FinderRemovalOwed, Owed, Settled);
two_state!(SnapshotNodeState, MidUpload, Settled);

#[cfg(test)]
mod tests {
    use super::*;

    /// Every flag keeps the bool it is built from, so the 0/1 written is unchanged.
    #[test]
    fn every_flag_round_trips_its_bool() {
        for flag in [true, false] {
            assert_eq!(Pinned::from(flag).as_bool(), flag);
            assert_eq!(MarkPending::from(flag).as_bool(), flag);
            assert_eq!(PayloadCompleted::from(flag).as_bool(), flag);
            assert_eq!(SignOutMarker::from(flag).as_bool(), flag);
            assert_eq!(FinderRemovalOwed::from(flag).as_bool(), flag);
            assert_eq!(SnapshotNodeState::from(flag).as_bool(), flag);
        }
    }
}
