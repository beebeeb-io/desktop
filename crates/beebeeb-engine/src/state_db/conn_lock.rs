//! dc-163: the state database's connection lock, re-entrant for the thread that holds a write batch.
//!
//! The first pull wrote one commit per row (2.1 M write calls at 41,510 rows, task dc-086). A
//! [`WriteBatch`] holds this lock on one thread across many rows and keeps one `BEGIN IMMEDIATE`
//! open; the rows' own `StateDb` calls on that thread re-enter the lock, and the transaction each
//! of them opens becomes a savepoint inside the batch. Every other thread waits for the batch to
//! commit, so nothing it writes ever joins (or is rolled back with) the batch.

use rusqlite::{Connection, Result, Savepoint, Transaction, TransactionBehavior};
use std::cell::{RefCell, RefMut};
use std::mem::ManuallyDrop;
use std::sync::{Condvar, LockResult, Mutex};
use std::thread::ThreadId;

/// Rows the first pull writes per commit.
pub const PULL_BATCH_ROWS: usize = 1_000;

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
        let _ = self
            .taken
            .wait_timeout_while(owner, std::time::Duration::from_millis(100), |o| o.3 == seen && o.2 > 0)
            .map(|_| ())
            .unwrap_or_else(|p| drop(p.into_inner()));
    }

    /// The connection, as `Mutex::lock` gave it (never poisoned). A thread that already holds a
    /// [`Hold`] gets it at once; any other waits.
    pub(crate) fn lock(&self) -> LockResult<ConnGuard<'_>> {
        self.acquire();
        Ok(ConnGuard {
            lock: self,
            conn: ManuallyDrop::new(self.conn.borrow_mut()),
        })
    }

    /// Holds the lock for this thread without borrowing the connection.
    pub(crate) fn hold(&self) -> Hold<'_> {
        self.acquire();
        Hold(self)
    }
}

/// The lock held by this thread with the connection free for its own calls.
pub(crate) struct Hold<'a>(&'a ConnLock);

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
/// stay (each holds whole rows only).
pub struct WriteBatch<'a> {
    lock: &'a ConnLock,
    hold: Option<Hold<'a>>,
    every: usize,
    rows: usize,
    commits: usize,
}

impl<'a> WriteBatch<'a> {
    pub(crate) fn open(lock: &'a ConnLock, every: usize) -> Result<Self> {
        let mut batch = Self {
            lock,
            hold: None,
            every: every.max(1),
            rows: 0,
            commits: 0,
        };
        batch.begin()?;
        Ok(batch)
    }

    fn begin(&mut self) -> Result<()> {
        let hold = self.lock.hold();
        self.lock.lock().expect("state_db lock").execute_batch("BEGIN IMMEDIATE")?;
        self.hold = Some(hold);
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        if self.hold.is_some() {
            self.lock.lock().expect("state_db lock").execute_batch("COMMIT")?;
            self.hold = None;
            self.commits += 1;
        }
        Ok(())
    }

    /// One row is written: commits the batch every `every` rows, letting other threads in between.
    /// `between` runs after such a commit, with the lock free (so it may wake another thread).
    pub fn row_done(&mut self, between: impl FnOnce(usize)) -> Result<()> {
        self.rows += 1;
        if self.rows % self.every == 0 {
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
