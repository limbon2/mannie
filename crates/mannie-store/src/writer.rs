//! The writer: one thread that owns the only read-write connection and runs
//! write jobs one at a time, each in its own transaction (ADR-0009).

use std::path::Path;
use std::thread::{self, JoinHandle};

use rusqlite::{Connection, Transaction, TransactionBehavior};
use tokio::sync::{mpsc, oneshot};

use crate::Durability;
use crate::error::{DatabaseError, StoreError};

/// How many write jobs may wait for the writer. A caller that finds the queue
/// full waits, which slows producers down instead of growing memory.
const QUEUE_LENGTH: usize = 256;

/// The read-write connection and the durability level it is set to.
struct WriterConnection {
    connection: Connection,
    synchronous: Option<Durability>,
}

impl WriterConnection {
    /// Sets SQLite's `synchronous` level for the next transaction. In WAL mode,
    /// FULL syncs the log on every commit, and NORMAL only at checkpoints.
    fn set_durability(&mut self, durability: Durability) -> rusqlite::Result<()> {
        if self.synchronous != Some(durability) {
            let level = match durability {
                Durability::Full => "FULL",
                Durability::Normal => "NORMAL",
            };
            self.connection.pragma_update(None, "synchronous", level)?;
            self.synchronous = Some(durability);
        }
        Ok(())
    }
}

type Job = Box<dyn FnOnce(&mut WriterConnection) + Send>;

/// Why a transaction did not commit: the work refused the change, or the store
/// failed. Either way, nothing of the transaction was written.
#[derive(Debug)]
pub enum TransactError<E> {
    /// The work found that the change is not allowed, such as a second active
    /// run in a conversation.
    Rejected(E),
    /// The store failed.
    Store(StoreError),
}

impl<E> From<rusqlite::Error> for TransactError<E> {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(StoreError::query(error))
    }
}

impl<E> From<StoreError> for TransactError<E> {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// The handle that submits write jobs to the writer thread.
///
/// Dropping it closes the queue and waits until the thread has finished the
/// queued jobs and closed the connection, so the database is closed when the
/// last store handle is gone.
#[derive(Debug)]
pub(crate) struct Writer {
    jobs: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl Writer {
    /// Starts the writer thread with the read-write connection.
    ///
    /// The writer overwrites deleted content with zeros (`secure_delete`), so
    /// text that the owner deleted does not linger in free pages of the file
    /// (security architecture, local data).
    pub(crate) fn start(connection: Connection, path: &Path) -> Result<Self, StoreError> {
        connection
            .pragma_update(None, "secure_delete", "ON")
            .map_err(|source| StoreError::Open {
                path: path.to_path_buf(),
                source: DatabaseError(source),
            })?;
        let (jobs, mut queue) = mpsc::channel::<Job>(QUEUE_LENGTH);
        let mut writer_connection = WriterConnection {
            connection,
            synchronous: None,
        };
        let thread = thread::Builder::new()
            .name("mannie-store-writer".to_owned())
            .spawn(move || {
                while let Some(job) = queue.blocking_recv() {
                    job(&mut writer_connection);
                }
                tracing::debug!("the store's writer stopped");
            })
            .map_err(|source| StoreError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        Ok(Self {
            jobs: Some(jobs),
            thread: Some(thread),
        })
    }

    /// Runs `work` in an immediate transaction on the writer thread and commits
    /// it with the given durability. The future resolves after the commit; a
    /// failed job is rolled back.
    pub(crate) async fn write<T, F>(&self, durability: Durability, work: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> rusqlite::Result<T> + Send + 'static,
    {
        self.submit(move |writer| run_in_transaction(writer, durability, work))
            .await?
    }

    /// Like [`Writer::write`], for work that can refuse its change: when `work`
    /// returns an error, the transaction is rolled back and the error returned.
    pub(crate) async fn transact<T, E, F>(
        &self,
        durability: Durability,
        work: F,
    ) -> Result<T, TransactError<E>>
    where
        T: Send + 'static,
        E: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<T, TransactError<E>> + Send + 'static,
    {
        self.submit(move |writer| transact_in(writer, durability, work))
            .await?
    }

    /// Runs `work` on the connection outside of any transaction, for statements
    /// that a transaction does not allow, such as `wal_checkpoint`.
    pub(crate) async fn maintain<T, F>(&self, work: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.submit(move |writer| work(&writer.connection).map_err(StoreError::query))
            .await?
    }

    /// Queues a job and waits for its result.
    async fn submit<T, F>(&self, job: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut WriterConnection) -> T + Send + 'static,
    {
        let jobs = self.jobs.as_ref().ok_or(StoreError::WriterStopped)?;
        let (reply, response) = oneshot::channel();
        let job: Job = Box::new(move |writer| {
            let result = job(writer);
            if reply.send(result).is_err() {
                tracing::debug!(
                    "a caller stopped waiting for its write; the write finished regardless"
                );
            }
        });
        jobs.send(job)
            .await
            .map_err(|_unsent_job| StoreError::WriterStopped)?;
        response.await.map_err(|_closed| StoreError::WriterStopped)
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // Closing the queue ends the thread's loop once the queued jobs ran.
        self.jobs.take();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the store's writer thread panicked");
        }
    }
}

fn run_in_transaction<T, F>(
    writer: &mut WriterConnection,
    durability: Durability,
    work: F,
) -> Result<T, StoreError>
where
    F: FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
{
    transact_in::<T, std::convert::Infallible, _>(writer, durability, |transaction| {
        work(transaction).map_err(TransactError::from)
    })
    .map_err(|error| match error {
        TransactError::Store(error) => error,
        TransactError::Rejected(never) => match never {},
    })
}

fn transact_in<T, E, F>(
    writer: &mut WriterConnection,
    durability: Durability,
    work: F,
) -> Result<T, TransactError<E>>
where
    F: FnOnce(&Transaction<'_>) -> Result<T, TransactError<E>>,
{
    writer.set_durability(durability)?;
    // Immediate: take the write lock at the start, so a busy database fails
    // before any work is done instead of at commit time.
    let transaction = writer
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Returning early drops the transaction, which rolls it back.
    let value = work(&transaction)?;
    transaction.commit()?;
    Ok(value)
}
