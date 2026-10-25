//! The files that the owner drops, which travel in parts (plugin ADR-0002,
//! decision 3): a JavaScript string ends near 1 GB, so one base64 string
//! cannot carry a large file. Only memory keeps the parts, until the upload
//! finishes, is cancelled, or gets no part for [`IDLE_MILLIS`].

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::commands::NotebooksError;

/// How long an upload waits for its next part before it is dropped.
pub(crate) const IDLE_MILLIS: i64 = 5 * 60 * 1000;

/// A file whose parts are arriving.
#[derive(Debug)]
struct Upload {
    notebook_id: String,
    name: String,
    bytes: Vec<u8>,
    /// The number of the part that comes next.
    next: u32,
    /// When the last part, or the start, arrived.
    touched: i64,
}

/// A file whose every part arrived.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Uploaded {
    pub(crate) notebook_id: String,
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
}

/// The uploads of a start of the plugin.
#[derive(Debug, Default)]
pub(crate) struct Uploads {
    entries: Mutex<HashMap<String, Upload>>,
}

impl Uploads {
    fn entries(&self) -> MutexGuard<'_, HashMap<String, Upload>> {
        // Every critical section only reads or replaces values.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Drops the uploads that got nothing for [`IDLE_MILLIS`] before `now`.
    fn drop_idle(entries: &mut HashMap<String, Upload>, now: i64) {
        entries.retain(|id, upload| {
            let idle = now.saturating_sub(upload.touched) >= IDLE_MILLIS;
            if idle {
                tracing::info!(upload = %id, "an upload that got no part for 5 minutes was dropped");
            }
            !idle
        });
    }

    /// Starts an upload of a file named `name` into a notebook, and returns
    /// its ID.
    pub(crate) fn start(&self, notebook_id: &str, name: &str, now: i64) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        let mut entries = self.entries();
        Self::drop_idle(&mut entries, now);
        entries.insert(
            id.clone(),
            Upload {
                notebook_id: notebook_id.to_owned(),
                name: name.to_owned(),
                bytes: Vec::new(),
                next: 0,
                touched: now,
            },
        );
        id
    }

    /// Adds the part numbered `index` to an upload. A part out of order ends
    /// the upload, since the file can no longer be whole.
    pub(crate) fn add(
        &self,
        id: &str,
        index: u32,
        part: &[u8],
        now: i64,
    ) -> Result<(), NotebooksError> {
        let mut entries = self.entries();
        Self::drop_idle(&mut entries, now);
        let upload = entries.get_mut(id).ok_or(NotebooksError::UploadNotFound)?;
        if index != upload.next {
            let expected = upload.next;
            entries.remove(id);
            return Err(NotebooksError::UploadPartOutOfOrder { expected });
        }
        upload.bytes.extend_from_slice(part);
        upload.next += 1;
        upload.touched = now;
        Ok(())
    }

    /// Ends an upload and gives its file.
    pub(crate) fn finish(&self, id: &str, now: i64) -> Result<Uploaded, NotebooksError> {
        let mut entries = self.entries();
        Self::drop_idle(&mut entries, now);
        let upload = entries.remove(id).ok_or(NotebooksError::UploadNotFound)?;
        Ok(Uploaded {
            notebook_id: upload.notebook_id,
            name: upload.name,
            bytes: upload.bytes,
        })
    }

    /// Drops an upload. Returns whether it existed.
    pub(crate) fn cancel(&self, id: &str) -> bool {
        self.entries().remove(id).is_some()
    }

    /// Drops the uploads into a notebook that was deleted.
    pub(crate) fn forget_notebook(&self, notebook_id: &str) {
        self.entries()
            .retain(|_id, upload| upload.notebook_id != notebook_id);
    }
}

#[cfg(test)]
mod tests {
    use super::{IDLE_MILLIS, Uploaded, Uploads};
    use crate::commands::NotebooksError;

    #[test]
    fn parts_in_order_make_the_file() {
        let uploads = Uploads::default();
        let id = uploads.start("n1", "guide.pdf", 0);

        uploads.add(&id, 0, b"%PDF", 10).unwrap();
        uploads.add(&id, 1, b"-1.4", 20).unwrap();

        assert_eq!(
            uploads.finish(&id, 30),
            Ok(Uploaded {
                notebook_id: "n1".to_owned(),
                name: "guide.pdf".to_owned(),
                bytes: b"%PDF-1.4".to_vec(),
            })
        );
        assert_eq!(uploads.finish(&id, 40), Err(NotebooksError::UploadNotFound));
    }

    #[test]
    fn a_part_out_of_order_ends_the_upload() {
        let uploads = Uploads::default();
        let id = uploads.start("n1", "guide.pdf", 0);
        uploads.add(&id, 0, b"a", 0).unwrap();

        assert_eq!(
            uploads.add(&id, 2, b"c", 0),
            Err(NotebooksError::UploadPartOutOfOrder { expected: 1 })
        );
        assert_eq!(
            uploads.add(&id, 1, b"b", 0),
            Err(NotebooksError::UploadNotFound)
        );
    }

    #[test]
    fn an_idle_upload_is_dropped_and_a_cancelled_one_is_gone() {
        let uploads = Uploads::default();
        let idle = uploads.start("n1", "old.pdf", 0);
        let busy = uploads.start("n1", "new.pdf", IDLE_MILLIS - 1);
        uploads.add(&busy, 0, b"x", IDLE_MILLIS - 1).unwrap();

        assert_eq!(
            uploads.add(&idle, 0, b"x", IDLE_MILLIS),
            Err(NotebooksError::UploadNotFound)
        );
        assert!(uploads.cancel(&busy));
        assert!(!uploads.cancel(&busy));
    }

    #[test]
    fn deleting_a_notebook_drops_its_uploads() {
        let uploads = Uploads::default();
        let gone = uploads.start("n1", "a.pdf", 0);
        let kept = uploads.start("n2", "b.pdf", 0);

        uploads.forget_notebook("n1");

        assert_eq!(
            uploads.finish(&gone, 0),
            Err(NotebooksError::UploadNotFound)
        );
        assert_eq!(
            uploads.finish(&kept, 0).map(|upload| upload.notebook_id),
            Ok("n2".to_owned())
        );
    }
}
