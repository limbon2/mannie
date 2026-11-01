//! A terminal's headless emulator
//! (plugins/terminal/docs/adr/0002-keep-a-headless-emulator-of-each-terminal-in-rust.md).
//!
//! vt100 keeps the screen and 200 rows of history, which a resize brings
//! back as a terminal does. A row that leaves the history goes to the text
//! log, through `on_scroll`, as a styled line that carries the attributes
//! that it inherits. Rows are numbered absolutely, from the first row that
//! the terminal had: the log's rows, then vt100's history, then the screen.
//! The setup's markers reach `unhandled_osc` at their place in the stream,
//! with the cursor's position, so a command's rows stay addressable after
//! they scroll.
//!
//! A full-screen program's rows, on the alternate screen, never reach the
//! log: they are no command's output.

use std::collections::VecDeque;
use std::num::NonZeroU16;

use vt100::capture::{
    BasicFormattedCaptureRange, BasicFormattedCaptureState, RowContents, basic_formatted_to_plain,
};

use crate::marks::{self, Mark};
use crate::sgr::Attributes;

/// The rows of history that vt100 keeps beyond the screen.
const HISTORY_ROWS: usize = 200;

/// The most bytes of a block's start that the emulator keeps while the
/// block runs, so that its first lines survive when the log lets them go.
pub(crate) const HEAD_BYTES: usize = 256 * 1024;

/// A place in a terminal's output: an absolute row and a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Position {
    /// The row, counted from the terminal's first row.
    pub(crate) row: u64,
    /// The column, from 0.
    pub(crate) column: u16,
}

/// A marker at its place in the output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlacedMark {
    /// What the marker says.
    pub(crate) mark: Mark,
    /// Where the cursor was.
    pub(crate) at: Position,
    /// For the start of a command, the command line between the prompt's
    /// end and here, as the screen shows it.
    pub(crate) line: Option<String>,
}

/// A row's plain text, and whether the next row continues it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RowText {
    /// The text, without trailing blanks.
    pub(crate) text: String,
    /// Whether the row wrapped onto the next one.
    pub(crate) wrapped: bool,
}

/// A row of the text log.
#[derive(Debug)]
struct LoggedRow {
    /// The row as styled text: the attributes that it inherits, then the
    /// capture's text.
    styled: Box<str>,
    /// Whether the next row continues it.
    wrapped: bool,
}

/// The rows that left vt100's history, the oldest first, within a size.
#[derive(Debug)]
struct TextLog {
    rows: VecDeque<LoggedRow>,
    /// The absolute number of the first row kept.
    first: u64,
    /// The bytes of the rows kept.
    bytes: usize,
    /// The most bytes kept.
    cap: usize,
}

impl TextLog {
    /// The absolute number of the row that the log takes next.
    fn end(&self) -> u64 {
        self.first + self.rows.len() as u64
    }

    fn push(&mut self, row: LoggedRow) {
        self.bytes += row.styled.len();
        self.rows.push_back(row);
        self.trim();
    }

    /// Lets the oldest rows go until the log fits its size.
    fn trim(&mut self) {
        while self.bytes > self.cap {
            let Some(oldest) = self.rows.pop_front() else {
                break;
            };
            self.bytes -= oldest.styled.len();
            self.first += 1;
        }
    }

    fn get(&self, row: u64) -> Option<&LoggedRow> {
        let index = usize::try_from(row.checked_sub(self.first)?).ok()?;
        self.rows.get(index)
    }
}

/// The first rows of a running block, as they leave the history.
#[derive(Debug, Default)]
pub(crate) struct Head {
    /// The block's first row.
    pub(crate) from: u64,
    /// The rows captured, by absolute number, in order.
    pub(crate) rows: Vec<(u64, RowText)>,
    bytes: usize,
}

/// What the emulator's callbacks collect.
#[derive(Debug)]
struct Capture {
    nonce: Box<[u8]>,
    log: TextLog,
    /// The capture's state across rows, which `on_scroll` continues.
    state: BasicFormattedCaptureState,
    /// The attributes at the end of the last logged row.
    attributes: Attributes,
    /// The markers found since the last `process` returned.
    marks: Vec<PlacedMark>,
    /// Where the last prompt ended, for the command line of the next
    /// command.
    prompt_end: Option<Position>,
    head: Option<Head>,
}

impl vt100::Callbacks for Capture {
    fn on_scroll(&mut self, contents: RowContents<'_>, alternate_screen: bool) {
        if alternate_screen {
            return;
        }
        let mut styled = String::new();
        // Writing to a String cannot fail.
        let _written = contents.write_formatted_basic(&mut styled, &mut self.state);
        // The capture starts a row with a line break when the row before it
        // ended its line; a row without one continues a wrapped line.
        let row = self.log.end();
        let text = if let Some(rest) = styled.strip_prefix('\n') {
            rest
        } else {
            if let Some(previous) = self.log.rows.back_mut() {
                previous.wrapped = true;
            }
            if let Some(head) = &mut self.head
                && let Some((last, text)) = head.rows.last_mut()
                && *last + 1 == row
            {
                text.wrapped = true;
            }
            styled.as_str()
        };
        let inherited = self.attributes.sequence();
        self.attributes.apply(text);
        if let Some(head) = &mut self.head
            && row >= head.from
            && head.bytes < HEAD_BYTES
        {
            let plain: String = basic_formatted_to_plain(text).collect();
            head.bytes += plain.len();
            head.rows.push((
                row,
                RowText {
                    text: plain.trim_end().to_owned(),
                    wrapped: false,
                },
            ));
        }
        let mut stored = inherited;
        stored.push_str(text);
        self.log.push(LoggedRow {
            styled: stored.into_boxed_str(),
            wrapped: false,
        });
    }

    fn unhandled_osc(&mut self, screen: &mut vt100::Screen, params: &[&[u8]]) {
        let Some(mark) = marks::parse(params, &self.nonce) else {
            return;
        };
        let (cursor_row, column) = screen.cursor_position();
        let top = self.log.end() + screen.scrollback_len() as u64;
        let at = Position {
            row: top + u64::from(cursor_row),
            column,
        };
        let line = match mark {
            Mark::PromptEnd => {
                self.prompt_end = Some(at);
                None
            }
            Mark::CommandStart => self
                .prompt_end
                .take()
                .and_then(|start| line_between(screen, top, start, at)),
            _ => None,
        };
        self.marks.push(PlacedMark { mark, at, line });
    }
}

/// The text between two places of the screen, such as a command line, or
/// `None` when the first place left the screen.
fn line_between(
    screen: &vt100::Screen,
    top: u64,
    start: Position,
    end: Position,
) -> Option<String> {
    let start_row = u16::try_from(start.row.checked_sub(top)?).ok()?;
    let end_row = u16::try_from(end.row.checked_sub(top)?).ok()?;
    let text = screen.contents_between(start_row, start.column, end_row, end.column);
    let line = text.trim().to_owned();
    Some(line)
}

/// A terminal's emulator.
pub(crate) struct Emulator {
    parser: vt100::Parser<Capture>,
    /// The history's length on the normal screen, as last seen there: the
    /// alternate screen has none of its own.
    normal_history: usize,
}

impl std::fmt::Debug for Emulator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The screen is the terminal's content, which stays out of logs.
        formatter
            .debug_struct("Emulator")
            .field("size", &self.size())
            .field("top_row", &self.top_row())
            .finish_non_exhaustive()
    }
}

/// A size in characters, at least one by one.
fn non_zero(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value).unwrap_or(NonZeroU16::MIN)
}

impl Emulator {
    /// An emulator of `columns` by `rows` whose markers carry `nonce`, and
    /// whose log keeps at most `log_cap` bytes.
    pub(crate) fn new(columns: u16, rows: u16, nonce: &str, log_cap: usize) -> Self {
        let capture = Capture {
            nonce: nonce.as_bytes().into(),
            log: TextLog {
                rows: VecDeque::new(),
                first: 0,
                bytes: 0,
                cap: log_cap,
            },
            state: BasicFormattedCaptureState::new(),
            attributes: Attributes::default(),
            marks: Vec::new(),
            prompt_end: None,
            head: None,
        };
        let parser = vt100::Parser::new_with_callbacks(
            non_zero(rows),
            non_zero(columns),
            HISTORY_ROWS,
            capture,
        );
        Self {
            parser,
            normal_history: 0,
        }
    }

    /// Feeds output to the emulator and returns the markers that it held,
    /// in order.
    pub(crate) fn process(&mut self, bytes: &[u8]) -> Vec<PlacedMark> {
        self.parser.process(bytes);
        if !self.parser.screen().alternate_screen() {
            self.normal_history = self.parser.screen().scrollback_len();
        }
        std::mem::take(&mut self.parser.callbacks_mut().marks)
    }

    /// Changes the screen's size. Rows that a smaller screen pushes out go
    /// to the history, and a larger screen brings them back.
    pub(crate) fn resize(&mut self, columns: u16, rows: u16) {
        self.parser.set_size(non_zero(rows), non_zero(columns));
        if !self.parser.screen().alternate_screen() {
            self.normal_history = self.parser.screen().scrollback_len();
        }
    }

    /// The screen's size: columns, then rows.
    pub(crate) fn size(&self) -> (u16, u16) {
        let (rows, columns) = self.parser.screen().size();
        (columns.get(), rows.get())
    }

    /// Changes the most bytes that the log keeps, and lets the oldest rows
    /// go to fit.
    pub(crate) fn set_log_cap(&mut self, cap: usize) {
        let log = &mut self.parser.callbacks_mut().log;
        log.cap = cap;
        log.trim();
    }

    /// The bytes that the log keeps.
    #[cfg(any(test, feature = "validation"))]
    pub(crate) fn log_bytes(&self) -> usize {
        self.parser.callbacks().log.bytes
    }

    /// Whether a full-screen program shows the alternate screen.
    pub(crate) fn alternate_screen(&self) -> bool {
        self.parser.screen().alternate_screen()
    }

    /// The absolute row of the normal screen's first row.
    pub(crate) fn top_row(&self) -> u64 {
        self.parser.callbacks().log.end() + self.normal_history as u64
    }

    /// The oldest row whose text the emulator still has.
    #[cfg(test)]
    pub(crate) fn first_row(&self) -> u64 {
        self.parser.callbacks().log.first
    }

    /// Where the cursor is.
    pub(crate) fn cursor(&self) -> Position {
        let (row, column) = self.parser.screen().cursor_position();
        Position {
            row: self.top_row() + u64::from(row),
            column,
        }
    }

    /// Starts keeping the first rows of a block that starts at `from`.
    pub(crate) fn start_head(&mut self, from: u64) {
        self.parser.callbacks_mut().head = Some(Head {
            from,
            ..Head::default()
        });
    }

    /// Stops keeping a block's first rows, and returns them.
    pub(crate) fn take_head(&mut self) -> Option<Head> {
        self.parser.callbacks_mut().head.take()
    }

    /// The plain text of the rows from `from` to before `to`, each `None`
    /// when the emulator no longer has it.
    pub(crate) fn rows_text(&mut self, from: u64, to: u64) -> Vec<Option<RowText>> {
        let log_end = self.parser.callbacks().log.end();
        let history_end = log_end + self.normal_history as u64;
        let mut texts = Vec::new();
        let mut row = from;
        while row < to && row < log_end {
            texts.push(self.parser.callbacks().log.get(row).map(|logged| {
                let plain: String = basic_formatted_to_plain(&logged.styled).collect();
                RowText {
                    text: plain.trim_end().to_owned(),
                    wrapped: logged.wrapped,
                }
            }));
            row += 1;
        }
        let alternate = self.alternate_screen();
        if row < to && row < history_end {
            let end = to.min(history_end);
            if alternate {
                texts.extend((row..end).map(|_| None));
            } else {
                let from_index = usize::try_from(row - log_end).unwrap_or(usize::MAX);
                let to_index = usize::try_from(end - log_end).unwrap_or(usize::MAX);
                texts.extend(self.history_text(from_index, to_index));
            }
            row = end;
        }
        if row < to {
            let screen = self.parser.screen();
            let (rows, columns) = screen.size();
            let visible: Vec<String> = if alternate {
                Vec::new()
            } else {
                screen.rows(0, columns.get()).collect()
            };
            while row < to {
                let index = row - history_end;
                let text = u16::try_from(index)
                    .ok()
                    .filter(|index| *index < rows.get())
                    .and_then(|index| {
                        visible.get(usize::from(index)).map(|text| RowText {
                            text: text.trim_end().to_owned(),
                            wrapped: screen.row_wrapped(index),
                        })
                    });
                texts.push(text);
                row += 1;
            }
        }
        texts
    }

    /// The rows of vt100's history from index `from` to before `to`, read a
    /// screen at a time through its scroll offset, which goes back to the
    /// live screen after.
    fn history_text(&mut self, from: usize, to: usize) -> Vec<Option<RowText>> {
        let mut texts = Vec::new();
        let length = self.parser.screen().scrollback_len();
        let (rows, columns) = self.parser.screen().size();
        let mut index = from;
        while index < to && index < length {
            self.parser.screen_mut().set_scrollback(length - index);
            let screen = self.parser.screen();
            let window = (to - index).min(usize::from(rows.get()));
            let visible: Vec<String> = screen.rows(0, columns.get()).take(window).collect();
            for (offset, text) in visible.into_iter().enumerate() {
                let visible_row = u16::try_from(offset).unwrap_or(u16::MAX);
                texts.push(Some(RowText {
                    text: text.trim_end().to_owned(),
                    wrapped: screen.row_wrapped(visible_row),
                }));
            }
            index += window;
        }
        self.parser.screen_mut().set_scrollback(0);
        while texts.len() < to.saturating_sub(from) {
            texts.push(None);
        }
        texts
    }

    /// A snapshot that rebuilds a view from nothing: at most `lines` of the
    /// log's latest rows, vt100's history and the screen, then the cursor,
    /// the input modes and the attributes in effect. The view resets before
    /// it writes the snapshot. Returns the snapshot and the absolute row of
    /// its first line.
    pub(crate) fn full_snapshot(&self, lines: usize) -> (String, u64) {
        let capture = self.parser.callbacks();
        let log = &capture.log;
        let take = lines.min(log.rows.len());
        let skip = log.rows.len() - take;
        let first_row = log.first + skip as u64;
        let mut snapshot = String::from("\x1b[m");
        let mut previous_wrapped = true;
        for row in log.rows.range(skip..) {
            if !previous_wrapped {
                snapshot.push_str("\r\n");
            }
            snapshot.push_str(&row.styled);
            previous_wrapped = row.wrapped;
        }
        let screen = self.parser.screen();
        if screen.alternate_screen() {
            // The normal screen's history and rows are out of reach while a
            // full-screen program shows; the log's rows go to the view's
            // scrollback, then the program's screen.
            if take > 0 {
                snapshot.push_str("\r\n");
            }
            let (rows, _columns) = screen.size();
            for _ in 1..rows.get() {
                snapshot.push_str("\r\n");
            }
            snapshot.push_str("\x1b[?1049h");
            snapshot.push_str(&screen.state_formatted());
            return (snapshot, first_row);
        }
        let mut tail = String::new();
        let mut state = capture.state.clone();
        if take == 0 {
            tail.push_str(&capture.attributes.sequence());
        }
        // Writing to a String cannot fail.
        let _written = screen.write_contents_formatted_basic(
            &mut tail,
            BasicFormattedCaptureRange::Full(&mut state),
        );
        let tail = if take == 0 {
            tail.strip_prefix('\n').unwrap_or(&tail).to_owned()
        } else {
            tail
        };
        snapshot.push_str(&tail.replace('\n', "\r\n"));
        snapshot.push_str("\x1b[m");
        snapshot.push_str(&screen.cursor_state_formatted());
        snapshot.push_str(&screen.input_mode_formatted());
        snapshot.push_str(&screen.attributes_formatted());
        (snapshot, first_row)
    }

    /// A snapshot of the screen alone, which a view that fell behind writes
    /// over its screen. Its scrollback stays as it was.
    pub(crate) fn screen_snapshot(&self) -> String {
        let screen = self.parser.screen();
        let mut snapshot = String::from(if screen.alternate_screen() {
            "\x1b[?1049h"
        } else {
            "\x1b[?1049l"
        });
        snapshot.push_str(&screen.state_formatted());
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::{Emulator, Position};
    use crate::marks::Mark;

    const NONCE: &str = "0123456789abcdef0123456789abcdef";

    fn marker(kind: &str) -> String {
        format!("\x1b]633;{kind};{NONCE}\x07")
    }

    fn texts(emulator: &mut Emulator, from: u64, to: u64) -> Vec<String> {
        emulator
            .rows_text(from, to)
            .into_iter()
            .map(|row| row.map_or_else(|| "<gone>".to_owned(), |row| row.text))
            .collect()
    }

    #[test]
    fn rows_keep_their_numbers_through_the_history_and_the_log() {
        let mut emulator = Emulator::new(20, 5, NONCE, 1 << 20);
        let mut output = String::new();
        for line in 1..=400 {
            let _written = write!(output, "line {line}\r\n");
        }
        emulator.process(output.as_bytes());

        // 400 lines and the cursor's empty row: 196 rows left the screen and
        // the 200 rows of history, so the screen starts at row 396.
        assert_eq!(emulator.top_row(), 396);
        assert_eq!(texts(&mut emulator, 0, 3), ["line 1", "line 2", "line 3"]);
        assert_eq!(
            texts(&mut emulator, 195, 198),
            ["line 196", "line 197", "line 198"]
        );
        assert_eq!(
            texts(&mut emulator, 397, 401),
            ["line 398", "line 399", "line 400", ""]
        );
    }

    #[test]
    fn a_resize_keeps_the_rows_where_they_were() {
        let mut emulator = Emulator::new(20, 5, NONCE, 1 << 20);
        emulator.process(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix\r\n");
        let before = texts(&mut emulator, 0, 7);

        emulator.resize(20, 10);
        let after = texts(&mut emulator, 0, 7);
        emulator.resize(20, 3);

        assert_eq!(before, after);
        assert_eq!(texts(&mut emulator, 0, 7), before);
    }

    #[test]
    fn markers_carry_their_place_and_a_command_its_line() {
        let mut emulator = Emulator::new(40, 5, NONCE, 1 << 20);
        let output = format!(
            "{}$ {}ls\r\n{}a.txt\r\nb.txt\r\n{}",
            marker("A"),
            marker("B"),
            marker("C"),
            marker("D;0")
        );

        let marks = emulator.process(output.as_bytes());

        let kinds: Vec<&Mark> = marks.iter().map(|placed| &placed.mark).collect();
        assert_eq!(
            kinds,
            [
                &Mark::PromptStart,
                &Mark::PromptEnd,
                &Mark::CommandStart,
                &Mark::CommandEnd { exit: Some(0) }
            ]
        );
        assert_eq!(marks[1].at, Position { row: 0, column: 2 });
        assert_eq!(marks[2].at, Position { row: 1, column: 0 });
        assert_eq!(marks[2].line.as_deref(), Some("ls"));
        assert_eq!(marks[3].at, Position { row: 3, column: 0 });
    }

    #[test]
    fn a_forged_marker_stays_output() {
        let mut emulator = Emulator::new(40, 5, NONCE, 1 << 20);

        let marks = emulator.process(b"\x1b]633;D;0;guessed\x07done\r\n");

        assert_eq!(marks, []);
        assert_eq!(texts(&mut emulator, 0, 1), ["done"]);
    }

    #[test]
    fn the_log_lets_its_oldest_rows_go() {
        let mut emulator = Emulator::new(20, 5, NONCE, 64);
        let mut output = String::new();
        for line in 1..=300 {
            let _written = write!(output, "line {line}\r\n");
        }
        emulator.process(output.as_bytes());

        assert!(emulator.log_bytes() <= 64);
        assert!(emulator.first_row() > 0);
        assert_eq!(texts(&mut emulator, 0, 1), ["<gone>"]);
    }

    #[test]
    fn a_full_snapshot_rebuilds_the_screen_and_the_rows_before_it() {
        let mut emulator = Emulator::new(20, 3, NONCE, 1 << 20);
        let mut output = String::from("\x1b[31mred\r\n");
        for line in 1..=300 {
            let _written = write!(output, "line {line}\r\n");
        }
        output.push_str("\x1b[mlast");
        emulator.process(output.as_bytes());

        let (snapshot, first_row) = emulator.full_snapshot(5);
        let mut view = Emulator::new(20, 3, NONCE, 1 << 20);
        view.process(snapshot.as_bytes());

        // 302 rows: the log holds rows 0 to 98, so its last 5 start at 94.
        assert_eq!(first_row, 94);
        assert_eq!(
            view.parser.screen().contents(),
            emulator.parser.screen().contents()
        );
        assert_eq!(view.cursor().column, emulator.cursor().column);
        // The first row of the snapshot carries the red that it inherits.
        assert!(snapshot.starts_with("\x1b[m\x1b[m\x1b[31m"));
    }
}
