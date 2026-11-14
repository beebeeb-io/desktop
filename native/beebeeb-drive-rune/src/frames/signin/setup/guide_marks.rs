//! Where the first-time guide's "done" is kept: once per ACCOUNT on this PC.
//!
//! Owner ruling 2026-11-13 (a deviation from the board, which says "Shown once per PC"; recorded in `FRAMES.md`):
//! an account that finished or skipped the guide does not get it again on this PC, also after a sign-out and a
//! sign-in; another account on the same PC gets it once. So the mark is asked of a source that answers for the
//! signed-in account and no other, and it is set when the guide is finished or skipped, never when it starts.
//!
//! A source answers late (the engine's reads its local state on a worker): [`GuideMarks::ask`] never waits, and the
//! guide's poll takes the answer in with [`GuideMarks::answer`]. Three answers, and only a definite "not done"
//! starts the guide.

/// What is known about the signed-in account and the guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Known {
    /// The account finished or skipped the guide on this PC.
    Done,
    /// It did not.
    NotDone,
    /// It cannot be said (nobody signed in, the answer could not be read): no guide starts.
    NotKnown,
}

pub trait GuideMarks {
    /// Ask whether the signed-in account has done the guide. Never waits. `account` is the account the window
    /// shows, for a source with nothing behind it; a source on the engine answers for the engine's own session.
    fn ask(&mut self, account: Option<&str>);
    /// The answer to the last [`Self::ask`], once, when it is there and still for the account that was asked for.
    /// Also the source's poll (it runs every frame while the app runs): it takes in what its workers answered and
    /// asks again for a mark that could not be kept. Never waits.
    fn answer(&mut self, account: Option<&str>) -> Option<Known>;
    /// The signed-in account finished or skipped the guide: keep that for it. Never waits.
    fn mark_done(&mut self, account: Option<&str>);
}

/// Nothing behind it: nothing is known, so no guide starts by itself, and nothing is kept. A build that ships
/// without the engine's source (`engine_host::guide`).
pub struct Unwired;

impl GuideMarks for Unwired {
    fn ask(&mut self, _account: Option<&str>) {}
    fn answer(&mut self, _account: Option<&str>) -> Option<Known> {
        None
    }
    fn mark_done(&mut self, _account: Option<&str>) {}
}

/// The marks in memory, per account: the board's sample window, and the tests.
#[derive(Default)]
pub struct FixtureMarks {
    done: Vec<String>,
    asked: Option<Option<String>>,
}

impl FixtureMarks {
    /// `account` has done the guide.
    pub fn done_for(account: &str) -> Self {
        Self {
            done: vec![account.to_string()],
            asked: None,
        }
    }

    #[cfg(test)]
    pub fn is_done(&self, account: &str) -> bool {
        self.done.iter().any(|a| a == account)
    }
}

impl GuideMarks for FixtureMarks {
    fn ask(&mut self, account: Option<&str>) {
        self.asked = Some(account.map(str::to_string));
    }
    fn answer(&mut self, account: Option<&str>) -> Option<Known> {
        let asked = self.asked.take()?;
        // Asked for another account than the one the window shows now: no answer.
        if asked.as_deref() != account {
            return None;
        }
        Some(match asked {
            None => Known::NotKnown,
            Some(account) if self.done.contains(&account) => Known::Done,
            Some(_) => Known::NotDone,
        })
    }
    fn mark_done(&mut self, account: Option<&str>) {
        if let Some(account) = account
            && !self.done.iter().any(|a| a == account)
        {
            self.done.push(account.to_string());
        }
    }
}

/// The source a window starts with: the in-memory marks where tooling runs, nothing in a build that ships (the
/// engine's is put in by `shell::state::init_on_engine`).
pub fn first(dev_build: bool) -> Box<dyn GuideMarks> {
    if dev_build {
        Box::new(FixtureMarks::default())
    } else {
        Box::new(Unwired)
    }
}
