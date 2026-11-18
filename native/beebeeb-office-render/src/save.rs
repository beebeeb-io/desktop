//! The serialised document, held by token between `SaveSize` and `SaveInto`.
//!
//! The helper serialises into its own memory on `SaveSize`, tells the app the length, and keeps the bytes until the
//! app has made a section of exactly that length and granted it. One copy is held at a time: a new `SaveSize` replaces
//! it (its token is stale from then on), and `SaveInto` takes it out, so the copy is dropped whether the copy into the
//! section works or not. The helper writes no file: the bytes only ever go into a section the app granted.

/// Why a `SaveInto` is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The token is not the one held (none held, or an older one). What is held stays held.
    Stale,
    /// The token is the one held, but the granted section is not its length. The copy is dropped: the app starts over
    /// with a new `SaveSize`.
    WrongLength,
}

/// The serialised copy awaiting its section.
#[derive(Default)]
pub(crate) struct Saves {
    held: Option<(u32, Vec<u8>)>,
    next: u32,
}

impl Saves {
    /// Holds `bytes`, replacing any earlier copy, and returns the token that names them.
    pub(crate) fn hold(&mut self, bytes: Vec<u8>) -> u32 {
        self.next = self.next.wrapping_add(1);
        self.held = Some((self.next, bytes));
        self.next
    }

    /// Takes the copy named `token` out for a section of `granted_len` bytes.
    pub(crate) fn take(&mut self, token: u32, granted_len: u64) -> Result<Vec<u8>, Refused> {
        match &self.held {
            Some((held, _)) if *held == token => {}
            _ => return Err(Refused::Stale),
        }
        let Some((_, bytes)) = self.held.take() else {
            return Err(Refused::Stale);
        };
        if bytes.len() as u64 != granted_len {
            return Err(Refused::WrongLength);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_is_taken_once_by_its_token() {
        let mut saves = Saves::default();
        let token = saves.hold(vec![1, 2, 3]);
        assert_eq!(saves.take(token.wrapping_add(1), 3), Err(Refused::Stale));
        assert_eq!(saves.take(token, 3), Ok(vec![1, 2, 3]));
        assert_eq!(saves.take(token, 3), Err(Refused::Stale), "dropped once taken");
    }

    #[test]
    fn a_newer_copy_makes_the_older_token_stale() {
        let mut saves = Saves::default();
        let old = saves.hold(vec![1]);
        let new = saves.hold(vec![2, 2]);
        assert_ne!(old, new);
        assert_eq!(saves.take(old, 1), Err(Refused::Stale));
        assert_eq!(saves.take(new, 2), Ok(vec![2, 2]));
    }

    #[test]
    fn a_wrong_length_drops_the_copy() {
        let mut saves = Saves::default();
        let token = saves.hold(vec![1, 2]);
        assert_eq!(saves.take(token, 5), Err(Refused::WrongLength));
        assert_eq!(saves.take(token, 2), Err(Refused::Stale));
    }
}
