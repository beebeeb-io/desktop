//! The live preview of a md file: the reader's body, parsed once per source (cached by a hash of it).

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rune::El;

use super::super::Ui;
use super::super::markdown::{self, Block};
use crate::theme::tokens::Theme;

thread_local! {
    /// The last source's hash and its blocks: typing re-parses once, a redraw parses nothing.
    static PARSED: RefCell<Option<(u64, Rc<Vec<Block>>)>> = const { RefCell::new(None) };
    #[cfg(test)]
    static PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn blocks_of(source: &str) -> Rc<Vec<Block>> {
    let mut h = DefaultHasher::new();
    source.hash(&mut h);
    let hash = h.finish();
    PARSED.with(|c| {
        if let Some((k, b)) = &*c.borrow()
            && *k == hash
        {
            return b.clone();
        }
        #[cfg(test)]
        PARSES.with(|n| n.set(n.get() + 1));
        let b = Rc::new(markdown::parse(source));
        *c.borrow_mut() = Some((hash, b.clone()));
        b
    })
}

pub fn view(source: &str, _ui: &Ui, t: &Theme) -> El {
    body(source, t)
}

fn body(source: &str, t: &Theme) -> El {
    markdown::body_in(&blocks_of(source), t, "live-preview-md-body")
}

#[cfg(test)]
#[path = "live_md_tests.rs"]
mod tests;
