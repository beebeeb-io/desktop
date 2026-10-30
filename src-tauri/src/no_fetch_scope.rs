//! Task 2284 A1.2: the no-fetch scope. While a scope is held, the Cloud Files FETCH_DATA callback
//! refuses this process's own fetch of the scoped identities; other processes are served.
//!
//! The registry is process-wide because the callback runs on threads a scope never sees. A scope
//! releases exactly the identities it entered, once, on every return path including a panic.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// How many live scopes hold each identity (a multiset, so two scopes for one file release
/// independently). Process-wide; poison-tolerant, as the registry must never wedge a fetch.
static SCOPED: LazyLock<Mutex<HashMap<String, usize>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn lock() -> std::sync::MutexGuard<'static, HashMap<String, usize>> {
    SCOPED.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A held no-fetch scope: until it drops, this process's own fetch of the entered identities is
/// refused.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) struct NoFetchScope {
    ids: Vec<String>,
}

impl Drop for NoFetchScope {
    fn drop(&mut self) {
        let mut scoped = lock();
        for id in &self.ids {
            match scoped.get_mut(id) {
                Some(count) if *count > 1 => *count -= 1,
                _ => {
                    scoped.remove(id);
                }
            }
        }
    }
}

/// Enter a no-fetch scope for `ids`: until the returned scope drops, this process's own fetch of
/// each identity is refused.
#[must_use]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn enter(ids: &[&str]) -> NoFetchScope {
    let mut scoped = lock();
    for id in ids {
        *scoped.entry((*id).to_owned()).or_insert(0) += 1;
    }
    NoFetchScope { ids: ids.iter().map(|id| (*id).to_owned()).collect() }
}

/// Whether a fetch of `identity` by `requester_pid` is refused: only this process's own fetch, and
/// only while a scope holds the identity.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn refuses(identity: &str, requester_pid: u32) -> bool {
    refuses_for(identity, requester_pid, std::process::id())
}

fn refuses_for(identity: &str, requester_pid: u32, own_pid: u32) -> bool {
    requester_pid == own_pid && lock().get(identity).is_some_and(|count| *count > 0)
}

#[cfg(test)]
mod tests {
    use super::{enter, refuses_for};

    #[test]
    fn a_scoped_id_is_refused_to_this_process_only() {
        let _scope = enter(&["s1-a"]);
        assert!(refuses_for("s1-a", 7, 7), "this process's own fetch is refused");
        assert!(!refuses_for("s1-a", 8, 7), "another process is served");
    }

    #[test]
    fn an_id_outside_every_scope_is_served() {
        let _scope = enter(&["s2-a"]);
        assert!(!refuses_for("s2-b", 7, 7), "an id outside the scope is served");
    }

    #[test]
    fn overlapping_scopes_release_independently() {
        let a = enter(&["s3"]);
        let b = enter(&["s3"]);
        drop(a);
        assert!(refuses_for("s3", 7, 7), "the second scope still holds it");
        drop(b);
        assert!(!refuses_for("s3", 7, 7), "the last scope released it");
    }

    #[test]
    fn a_scope_ends_on_every_return_path() {
        fn early_return() -> Result<(), ()> {
            let _scope = enter(&["s4-early"]);
            assert!(refuses_for("s4-early", 7, 7), "held inside the scope");
            Err(())
        }

        assert_eq!(early_return(), Err(()));
        assert!(!refuses_for("s4-early", 7, 7), "released on the early return");

        let panicked = std::panic::catch_unwind(|| {
            let _scope = enter(&["s4-panic"]);
            assert!(refuses_for("s4-panic", 7, 7), "held inside the scope");
            panic!("inside the scope");
        });
        assert!(panicked.is_err());
        assert!(!refuses_for("s4-panic", 7, 7), "released while unwinding");
    }
}
