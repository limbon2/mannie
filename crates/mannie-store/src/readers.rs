//! The reader pool: a few read-only connections that serve reads in parallel
//! with the writer, which WAL mode allows (ADR-0009).

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use rusqlite::{Connection, OpenFlags};
use tokio::sync::Semaphore;

use crate::error::{DatabaseError, StoreError};

/// Read-only connections, each lent to one read at a time.
#[derive(Debug)]
pub(crate) struct Readers {
    idle: Mutex<Vec<Connection>>,
    permits: Arc<Semaphore>,
}

impl Readers {
    /// Opens `count` read-only connections to the database.
    pub(crate) fn open(path: &Path, count: usize) -> Result<Self, StoreError> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut connections = Vec::with_capacity(count);
        for _ in 0..count {
            let opened = Connection::open_with_flags(path, flags).and_then(|connection| {
                crate::store::configure_connection(&connection)?;
                Ok(connection)
            });
            let connection = opened.map_err(|source| StoreError::Open {
                path: path.to_path_buf(),
                source: DatabaseError(source),
            })?;
            connections.push(connection);
        }
        Ok(Self {
            idle: Mutex::new(connections),
            permits: Arc::new(Semaphore::new(count)),
        })
    }

    /// Runs `work` on an idle read-only connection, on a blocking thread.
    pub(crate) async fn read<T, F>(self: &Arc<Self>, work: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_closed| StoreError::ReadersStopped)?;
        let readers = Arc::clone(self);
        let joined = tokio::task::spawn_blocking(move || {
            // Each permit stands for one idle connection, so holding a permit
            // means a connection is available.
            let lent = LentConnection::take(&readers);
            let result = work(lent.connection()).map_err(StoreError::query);
            drop(lent);
            drop(permit);
            result
        })
        .await;
        match joined {
            Ok(result) => result,
            // A panic in a read is a bug; it reaches the caller unchanged.
            Err(join_error) if join_error.is_panic() => {
                std::panic::resume_unwind(join_error.into_panic())
            }
            // The runtime is shutting down and cancelled the read.
            Err(_cancelled) => Err(StoreError::ReadersStopped),
        }
    }
}

/// A connection taken from the pool. It goes back when dropped, also when a
/// read panics, so the pool never shrinks.
struct LentConnection<'pool> {
    readers: &'pool Readers,
    connection: Option<Connection>,
}

impl<'pool> LentConnection<'pool> {
    fn take(readers: &'pool Readers) -> Self {
        let connection = readers
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        Self {
            readers,
            connection,
        }
    }

    fn connection(&self) -> &Connection {
        self.connection
            .as_ref()
            .expect("a reader holds a permit, and every permit stands for one idle connection")
    }
}

impl Drop for LentConnection<'_> {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            self.readers
                .idle
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(connection);
        }
    }
}
