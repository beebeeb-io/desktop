//! dc-163: the state database's connection lock, re-entrant for the thread that holds a write batch.
//!
//! The first pull wrote one commit per row (2.1 M write calls at 41,510 rows, task dc-086). A
//! [`WriteBatch`] holds this lock on one thread across many rows and keeps one `BEGIN IMMEDIATE`
//! open; the rows' own `StateDb` calls on that thread re-enter the lock, and the transaction each
//! of them opens becomes a savepoint inside the batch. Every other thread waits for the batch to
//! commit, so nothing it writes ever joins (or is rolled back with) the batch.

use rusqlite::{Connection, Result, Savepoint, Transaction, TransactionBehavior};
use std::cell::{RefCell, RefMut};
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::sync::{Condvar, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

/// RE-a: statements kept compiled on the connection (`prepare_cached`).
const STATEMENT_CACHE: usize = 128;

/// Rows the first pull writes per commit, at most.
pub const PULL_BATCH_ROWS: usize = 1_000;

/// dc-170: the longest a write batch stays open before it commits at the next whole row, whatever its row count.
/// Other `StateDb` connections wait 5 s (busy timeout) for the file lock, and threads on this one wait for the
/// batch, so a batch is kept far under that.
pub const BATCH_MAX_TIME: Duration = Duration::from_millis(250);

/// dc-170: `lock()` on a thread whose own [`ConnGuard`] is still alive. The lock is left as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReEntered;

impl std::fmt::Display for ReEntered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("state_db connection already borrowed on this thread")
    }
}

impl std::error::Error for ReEntered {}

/// The connection, behind a lock one thread may take more than once.
pub(crate) struct ConnLock {
    /// The holding thread and how many times it holds the lock.
    /// (holder, depth, threads waiting, times a waiting thread took it): dc-164 needs the last two.
    owner: Mutex<(Option<ThreadId>, usize, usize, u64)>,
    freed: Condvar,
    /// dc-164: a waiting thread took the lock.
    taken: Condvar,
    conn: RefCell<Connection>,
}

// SAFETY: `conn` is reached only by the thread recorded in `owner` (see `acquire`), so the
// `RefCell` is never touched by two threads at once. `Connection` itself is `Send`.
unsafe impl Sync for ConnLock {}

impl ConnLock {
    pub(crate) fn new(conn: Connection) -> Self {
        // RE-a: a pull's per-row statements are cached (`prepare_cached`), not compiled again for each of a 42k-row
        // first listing's rows; rusqlite keeps 16 by default, fewer than one row's ingest uses.
        conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE);
        Self {
            owner: Mutex::new((None, 0, 0, 0)),
            freed: Condvar::new(),
            taken: Condvar::new(),
            conn: RefCell::new(conn),
        }
    }

    fn acquire(&self) {
        let me = std::thread::current().id();
        let mut owner = self.owner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            match owner.0 {
                Some(holder) if holder == me => break,
                None => {
                    owner.0 = Some(me);
                    break;
                }
                Some(_) => {
                    owner.2 += 1;
                    owner = self.freed.wait(owner).unwrap_or_else(std::sync::PoisonError::into_inner);
                    owner.2 -= 1;
                    if owner.0.is_none() {
                        owner.0 = Some(me);
                        owner.3 += 1;
                        self.taken.notify_all();
                        break;
                    }
                }
            }
        }
        owner.1 += 1;
    }

    fn release(&self) {
        let mut owner = self.owner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        owner.1 -= 1;
        if owner.1 == 0 {
            owner.0 = None;
            drop(owner);
            self.freed.notify_one();
        }
    }

    /// dc-164: called with the lock free (between two batches): when another thread waits for it, let one take it
    /// before this thread takes it again. A batch thread otherwise takes it back before the woken thread runs, and
    /// every other reader (the early placeholder passes, the status line) waits for the whole pull. Bounded: after
    /// 100 ms it goes on regardless.
    fn let_one_in(&self) {
        let owner = self.owner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if owner.2 == 0 || owner.0.is_some() {
            return;
        }
        let seen = owner.3;
        self.taken
            .wait_timeout_while(owner, std::time::Duration::from_millis(100), |o| o.3 == seen && o.2 > 0)
            .map(|_| ())
            .unwrap_or_else(|p| drop(p.into_inner()));
    }

    /// The connection. A thread that already holds a [`Hold`] gets it at once; any other waits.
    /// dc-170: a thread whose own [`ConnGuard`] is still alive gets [`ReEntered`], with the lock's
    /// depth as it was (it panicked inside `borrow_mut` with the depth raised: the lock held forever).
    pub(crate) fn lock(&self) -> std::result::Result<ConnGuard<'_>, ReEntered> {
        self.acquire();
        match self.conn.try_borrow_mut() {
            Ok(conn) => Ok(ConnGuard {
                lock: self,
                conn: ManuallyDrop::new(conn),
            }),
            Err(_) => {
                self.release();
                Err(ReEntered)
            }
        }
    }

    /// Holds the lock for this thread without borrowing the connection.
    pub(crate) fn hold(&self) -> Hold<'_> {
        self.acquire();
        Hold(self, PhantomData)
    }
}

/// The lock held by this thread with the connection free for its own calls.
/// dc-170: `!Send` (the raw pointer): the lock is keyed by thread, so a hold released on another
/// thread would free the lock under its owner and leave the owner's depth wrong.
pub(crate) struct Hold<'a>(&'a ConnLock, PhantomData<*const ()>);

impl Drop for Hold<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// The connection while the lock is held.
pub struct ConnGuard<'a> {
    lock: &'a ConnLock,
    conn: ManuallyDrop<RefMut<'a, Connection>>,
}

impl Drop for ConnGuard<'_> {
    fn drop(&mut self) {
        // The borrow ends before another thread can take the lock.
        // SAFETY: dropped once, here, and never used after.
        unsafe { ManuallyDrop::drop(&mut self.conn) };
        self.lock.release();
    }
}

impl std::ops::Deref for ConnGuard<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.conn
    }
}

impl std::ops::DerefMut for ConnGuard<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

impl ConnGuard<'_> {
    /// `Connection::transaction_with_behavior`, or a savepoint when a write batch is open.
    pub(crate) fn transaction_with_behavior(&mut self, behavior: TransactionBehavior) -> Result<WriteTx<'_>> {
        if self.conn.is_autocommit() {
            Ok(WriteTx::Tx(self.conn.transaction_with_behavior(behavior)?))
        } else {
            Ok(WriteTx::Sp(self.conn.savepoint()?))
        }
    }

    /// `Connection::transaction`, or a savepoint when a write batch is open.
    pub(crate) fn transaction(&mut self) -> Result<WriteTx<'_>> {
        self.transaction_with_behavior(TransactionBehavior::Deferred)
    }
}

/// A transaction of its own, or a savepoint inside the open write batch.
pub enum WriteTx<'c> {
    Tx(Transaction<'c>),
    Sp(Savepoint<'c>),
}

impl WriteTx<'_> {
    pub(crate) fn commit(self) -> Result<()> {
        match self {
            Self::Tx(tx) => tx.commit(),
            Self::Sp(sp) => sp.commit(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn rollback(self) -> Result<()> {
        match self {
            Self::Tx(tx) => tx.rollback(),
            Self::Sp(mut sp) => sp.rollback(),
        }
    }
}

impl std::ops::Deref for WriteTx<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        match self {
            Self::Tx(tx) => tx,
            Self::Sp(sp) => sp,
        }
    }
}

/// dc-163: rows written in batches of `every`, one commit each, on the thread that opened it.
/// Other threads wait while a batch is open. Dropped without [`WriteBatch::finish`], the open
/// batch is rolled back, so a failed pull never leaves half a batch; batches already committed
/// stay (each holds whole rows only). dc-170: a batch also commits once it has been open
/// [`BATCH_MAX_TIME`], at the next whole row. `!Send`, through its [`Hold`] (and `_bound`, when no
/// hold is open), as the lock is keyed by thread.
pub struct WriteBatch<'a> {
    lock: &'a ConnLock,
    hold: Option<Hold<'a>>,
    every: usize,
    max_time: Duration,
    rows: usize,
    /// Rows written since the open batch began, and when it began.
    batch_rows: usize,
    opened: Instant,
    commits: usize,
    /// The longest a batch was open, and its rows.
    longest: (Duration, usize),
    _bound: PhantomData<*const ()>,
}

impl<'a> WriteBatch<'a> {
    pub(crate) fn open(lock: &'a ConnLock, every: usize) -> Result<Self> {
        Self::open_capped(lock, every, BATCH_MAX_TIME)
    }

    pub(crate) fn open_capped(lock: &'a ConnLock, every: usize, max_time: Duration) -> Result<Self> {
        let mut batch = Self {
            lock,
            hold: None,
            every: every.max(1),
            max_time,
            rows: 0,
            batch_rows: 0,
            opened: Instant::now(),
            commits: 0,
            longest: (Duration::ZERO, 0),
            _bound: PhantomData,
        };
        batch.begin()?;
        Ok(batch)
    }

    fn begin(&mut self) -> Result<()> {
        let hold = self.lock.hold();
        self.lock.lock().expect("state_db lock").execute_batch("BEGIN IMMEDIATE")?;
        self.hold = Some(hold);
        self.batch_rows = 0;
        self.opened = Instant::now();
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        if self.hold.is_some() {
            self.lock.lock().expect("state_db lock").execute_batch("COMMIT")?;
            self.hold = None;
            self.commits += 1;
            let took = self.opened.elapsed();
            tracing::debug!(rows = self.batch_rows, ms = took.as_millis() as u64, "state_db write batch committed");
            if took > self.longest.0 {
                self.longest = (took, self.batch_rows);
            }
        }
        Ok(())
    }

    /// One row is written: commits the batch every `every` rows or once it has been open
    /// `max_time`, letting other threads in between. `between` runs after such a commit, with the
    /// lock free (so it may wake another thread).
    pub fn row_done(&mut self, between: impl FnOnce(usize)) -> Result<()> {
        self.rows += 1;
        self.batch_rows += 1;
        if self.batch_rows >= self.every || self.opened.elapsed() >= self.max_time {
            self.commit()?;
            between(self.rows);
            self.lock.let_one_in();
            self.begin()?;
        }
        Ok(())
    }

    /// Commits what is open. Returns the number of commits the batch made.
    pub fn finish(mut self) -> Result<usize> {
        self.commit()?;
        if self.commits > 1 {
            tracing::info!(
                rows = self.rows,
                commits = self.commits,
                longest_ms = self.longest.0.as_millis() as u64,
                longest_rows = self.longest.1,
                "state_db write batch finished"
            );
        }
        Ok(self.commits)
    }
}

impl Drop for WriteBatch<'_> {
    fn drop(&mut self) {
        if self.hold.is_some() {
            let conn = self.lock.lock().expect("state_db lock");
            if !conn.is_autocommit() {
                let _ = conn.execute_batch("ROLLBACK");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock_with_table() -> ConnLock {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (n INTEGER)").unwrap();
        ConnLock::new(conn)
    }

    fn depth(lock: &ConnLock) -> (Option<ThreadId>, usize) {
        let o = lock.owner.lock().unwrap();
        (o.0, o.1)
    }

    /// dc-170 (1): slow rows commit by time, long before the row count is reached.
    #[test]
    fn batch_commits_once_open_max_time_whatever_its_rows() {
        let lock = lock_with_table();
        let mut batch = WriteBatch::open_capped(&lock, 1_000, Duration::from_millis(30)).unwrap();
        for n in 0..6 {
            if n > 0 {
                batch.row_done(|_| {}).unwrap();
            }
            lock.lock().unwrap().execute("INSERT INTO t VALUES (?1)", [n]).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        let longest = batch.longest.0;
        let commits = batch.finish().unwrap();
        assert!(commits >= 3, "{commits} commits for 6 rows of 20 ms under a 30 ms cap");
        assert!(longest < Duration::from_millis(30 + 20 + 100), "a batch was open {longest:?}");
        let rows: i64 = lock.lock().unwrap().query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 6);
        assert_eq!(depth(&lock), (None, 0));
    }

    /// dc-170 (1): fast rows still commit by count.
    #[test]
    fn batch_still_commits_every_n_rows() {
        let lock = lock_with_table();
        let mut batch = WriteBatch::open_capped(&lock, 10, Duration::from_secs(60)).unwrap();
        for n in 0..25 {
            if n > 0 {
                batch.row_done(|_| {}).unwrap();
            }
            lock.lock().unwrap().execute("INSERT INTO t VALUES (?1)", [n]).unwrap();
        }
        assert_eq!(batch.finish().unwrap(), 3);
    }

    /// dc-170 (2): `lock()` under a live guard on the same thread is an error, and the lock is
    /// free for another thread once the guard goes (it panicked with the depth raised: held forever).
    #[test]
    fn same_thread_reentry_under_a_live_guard_is_an_error_and_leaks_nothing() {
        let lock = std::sync::Arc::new(lock_with_table());
        let guard = lock.lock().unwrap();
        assert_eq!(lock.lock().err(), Some(ReEntered));
        assert_eq!(depth(&lock).1, 1, "the failed lock() changed the depth");
        drop(guard);
        assert_eq!(depth(&lock), (None, 0));

        let (tx, rx) = std::sync::mpsc::channel();
        let other = lock.clone();
        std::thread::spawn(move || {
            let n: i64 = other.lock().unwrap().query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0)).unwrap();
            tx.send(n).unwrap();
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(0), "another thread never got the lock");
    }

    /// dc-170 (2): re-entry inside a batch (under the batch's guard) leaves the batch usable.
    #[test]
    fn reentry_inside_a_batch_leaves_the_batch_whole() {
        let lock = lock_with_table();
        let batch = WriteBatch::open(&lock, 10).unwrap();
        {
            let guard = lock.lock().unwrap();
            assert!(lock.lock().is_err());
            guard.execute("INSERT INTO t VALUES (1)", []).unwrap();
        }
        assert_eq!(batch.finish().unwrap(), 1);
        assert_eq!(depth(&lock), (None, 0));
    }

    /// dc-170 (3): `Hold` and `WriteBatch` must not leave their thread. Does not compile when
    /// either is `Send` (the two impls below then both apply and the call is ambiguous).
    #[test]
    fn hold_and_write_batch_are_not_send() {
        trait AmbiguousIfSend<A> {
            fn check() {}
        }
        impl<T: ?Sized> AmbiguousIfSend<()> for T {}
        impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}
        <Hold<'static> as AmbiguousIfSend<_>>::check();
        <WriteBatch<'static> as AmbiguousIfSend<_>>::check();
        <ConnGuard<'static> as AmbiguousIfSend<_>>::check();
    }
}
