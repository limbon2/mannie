//! The size cap: each day's log file stops growing at a fixed size, so a
//! runaway loop cannot fill the disk (engineering standards, observability).

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

use mannie_types::clock::Clock;
use time::OffsetDateTime;
use time::macros::format_description;

/// The line written once when a day's file reaches the cap.
const CAP_NOTICE: &str = concat!(
    r#"{"level":"WARN","target":"mannie_logging","fields":{"message":"#,
    r#""the log file reached its size limit for today; later lines of this day are dropped"}}"#,
    "\n"
);

/// Writes to the daily log file until the day's file reaches `limit` bytes,
/// then drops lines until the next day. The day is the UTC date, which is also
/// the date in the file name.
pub(crate) struct CappedWriter<W> {
    inner: W,
    limit: u64,
    clock: Arc<dyn Clock>,
    directory: PathBuf,
    file_prefix: String,
    day: Option<String>,
    written: u64,
    noticed: bool,
}

impl<W: Write> CappedWriter<W> {
    pub(crate) fn new(
        inner: W,
        limit: u64,
        clock: Arc<dyn Clock>,
        directory: PathBuf,
        file_prefix: String,
    ) -> Self {
        Self {
            inner,
            limit,
            clock,
            directory,
            file_prefix,
            day: None,
            written: 0,
            noticed: false,
        }
    }

    /// Starts counting anew when the date changed, from the size the day's
    /// file already has, so a restart does not reset the cap.
    fn roll_day(&mut self) {
        let today = utc_date(self.clock.now().as_millis());
        if self.day.as_deref() == Some(today.as_str()) {
            return;
        }
        let file = self
            .directory
            .join(format!("{}.{today}.log", self.file_prefix));
        self.written = fs::metadata(file).map_or(0, |metadata| metadata.len());
        self.noticed = self.written >= self.limit;
        self.day = Some(today);
    }
}

impl<W: Write> Write for CappedWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.roll_day();
        let length = buffer.len() as u64;
        if self.written.saturating_add(length) <= self.limit {
            self.inner.write_all(buffer)?;
            self.written += length;
        } else if !self.noticed {
            self.noticed = true;
            self.inner.write_all(CAP_NOTICE.as_bytes())?;
            self.written = self.written.saturating_add(CAP_NOTICE.len() as u64);
        }
        // Dropped lines count as written: the logger must not retry them.
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// The UTC date in the format of tracing-appender's daily file names.
fn utc_date(millis: i64) -> String {
    let format = format_description!("[year]-[month]-[day]");
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(millis) * 1_000_000)
        .ok()
        .and_then(|moment| moment.format(&format).ok())
        .unwrap_or_else(|| "unknown-date".to_owned())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    use mannie_types::clock::{Clock, UtcMillis};

    use super::{CAP_NOTICE, CappedWriter, utc_date};

    #[derive(Debug)]
    struct TestClock(AtomicI64);

    impl Clock for TestClock {
        fn now(&self) -> UtcMillis {
            UtcMillis::from_millis(self.0.load(Ordering::SeqCst))
        }
    }

    /// 2026-10-13T12:00:00Z
    const NOON: i64 = 1_791_892_800_000;
    const DAY: i64 = 86_400_000;

    fn write_lines(writer: &mut CappedWriter<&mut Vec<u8>>, count: usize) {
        for _ in 0..count {
            std::io::Write::write_all(writer, b"0123456789\n").unwrap();
        }
    }

    #[test]
    fn stops_at_the_limit_with_one_notice_and_resumes_the_next_day() {
        let directory = tempfile::tempdir().unwrap();
        let clock = Arc::new(TestClock(AtomicI64::new(NOON)));
        let mut output = Vec::new();
        {
            let mut writer = CappedWriter::new(
                &mut output,
                33,
                Arc::<TestClock>::clone(&clock),
                directory.path().into(),
                "t".into(),
            );
            write_lines(&mut writer, 5);
            clock.0.fetch_add(DAY, Ordering::SeqCst);
            write_lines(&mut writer, 1);
        }

        let expected = format!(
            "{}{}{}",
            "0123456789\n".repeat(3),
            CAP_NOTICE,
            "0123456789\n"
        );
        assert_eq!(String::from_utf8(output).unwrap(), expected);
    }

    #[test]
    fn counts_what_the_day_file_already_holds() {
        let directory = tempfile::tempdir().unwrap();
        let today = utc_date(NOON);
        fs::write(
            directory.path().join(format!("t.{today}.log")),
            "x".repeat(30),
        )
        .unwrap();
        let clock = Arc::new(TestClock(AtomicI64::new(NOON)));
        let mut output = Vec::new();
        {
            let mut writer =
                CappedWriter::new(&mut output, 33, clock, directory.path().into(), "t".into());
            write_lines(&mut writer, 1);
        }

        assert_eq!(String::from_utf8(output).unwrap(), CAP_NOTICE);
    }

    #[test]
    fn dates_match_the_daily_file_names() {
        assert_eq!(utc_date(NOON), "2026-10-13");
        assert_eq!(utc_date(NOON + 12 * 3_600_000), "2026-10-14");
    }
}
