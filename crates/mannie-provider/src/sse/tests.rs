use proptest::prelude::*;

use super::{MAX_EVENT_BYTES, SseError, SseEvent, SseParser};

fn parse_all(chunks: &[&[u8]]) -> Result<Vec<SseEvent>, SseError> {
    let mut parser = SseParser::new();
    let mut events = Vec::new();
    for chunk in chunks {
        events.extend(parser.push(chunk)?);
    }
    Ok(events)
}

fn event(event: &str, data: &str) -> SseEvent {
    SseEvent {
        event: event.to_owned(),
        data: data.to_owned(),
    }
}

#[test]
fn events_are_separated_by_blank_lines() {
    let stream = b"data: first\n\nevent: update\ndata: second\n\n";

    let events = parse_all(&[stream]).unwrap();

    assert_eq!(
        events,
        [event("message", "first"), event("update", "second")]
    );
}

#[test]
fn every_kind_of_line_ending_works() {
    let stream = b"data: one\r\n\r\ndata: two\r\rdata: three\n\n";

    let events = parse_all(&[stream]).unwrap();

    assert_eq!(
        events,
        [
            event("message", "one"),
            event("message", "two"),
            event("message", "three")
        ]
    );
}

#[test]
fn a_crlf_split_between_chunks_is_one_line_ending() {
    let events = parse_all(&[b"data: one\r", b"\n\r", b"\ndata: two\n\n"]).unwrap();

    assert_eq!(events, [event("message", "one"), event("message", "two")]);
}

#[test]
fn data_lines_join_with_line_feeds_and_keep_spaces_after_the_first() {
    let events = parse_all(&[b"data:  indented\ndata\ndata:last\n\n"]).unwrap();

    assert_eq!(events, [event("message", " indented\n\nlast")]);
}

#[test]
fn comments_ids_and_retries_are_skipped() {
    let stream = b": keepalive\nid: 7\nretry: 1000\ndata: kept\n\n: OPENROUTER PROCESSING\n\n";

    let events = parse_all(&[stream]).unwrap();

    assert_eq!(events, [event("message", "kept")]);
}

#[test]
fn an_event_without_data_is_not_dispatched_but_an_empty_data_field_is() {
    let events = parse_all(&[b"event: ping\n\ndata:\n\n"]).unwrap();

    assert_eq!(events, [event("message", "")]);
}

#[test]
fn a_byte_order_mark_at_the_start_is_skipped() {
    let events = parse_all(&[b"\xEF\xBB\xBFdata: x\n\n"]).unwrap();

    assert_eq!(events, [event("message", "x")]);
}

#[test]
fn utf8_split_between_chunks_survives() {
    let text = "data: grüße 🙂\n\n".as_bytes();
    let (first, second) = text.split_at(10);

    let events = parse_all(&[first, second]).unwrap();

    assert_eq!(events, [event("message", "grüße 🙂")]);
}

#[test]
fn invalid_utf8_is_refused() {
    assert_eq!(parse_all(&[b"data: \xFF\n\n"]), Err(SseError::InvalidUtf8));
}

#[test]
fn an_event_over_the_limit_is_refused() {
    let line = format!("data: {}\n", "a".repeat(MAX_EVENT_BYTES / 2));
    let stream = format!("{line}{line}{line}\n");

    assert_eq!(parse_all(&[stream.as_bytes()]), Err(SseError::TooLarge));
}

#[test]
fn a_stream_cut_in_the_middle_of_an_event_is_noticed() {
    let mut parser = SseParser::new();
    parser.push(b"data: complete\n\n").unwrap();
    assert!(!parser.has_partial_event());

    parser.push(b"data: cut").unwrap();

    assert!(parser.has_partial_event());
}

/// A stream of events as a server could write it, with every kind of line
/// ending and comments in between.
fn stream_strategy() -> impl Strategy<Value = (Vec<u8>, Vec<SseEvent>)> {
    let field_text = "[a-zA-Z0-9 äöü🙂{}\":,._-]{0,40}";
    let event = (
        proptest::option::of("[a-z_.]{1,20}"),
        proptest::collection::vec(field_text, 1..4),
        prop_oneof![Just("\n"), Just("\r\n"), Just("\r")],
        any::<bool>(),
    );
    proptest::collection::vec(event, 0..12).prop_map(|events| {
        let mut bytes = Vec::new();
        let mut expected = Vec::new();
        for (name, lines, ending, comment) in events {
            if comment {
                bytes.extend_from_slice(format!(": keepalive{ending}").as_bytes());
            }
            if let Some(name) = &name {
                bytes.extend_from_slice(format!("event: {name}{ending}").as_bytes());
            }
            for line in &lines {
                bytes.extend_from_slice(format!("data: {line}{ending}").as_bytes());
            }
            bytes.extend_from_slice(ending.as_bytes());
            expected.push(SseEvent {
                event: name.unwrap_or_else(|| "message".to_owned()),
                data: lines.join("\n"),
            });
        }
        (bytes, expected)
    })
}

proptest! {
    #[test]
    fn any_split_into_chunks_gives_the_same_events(
        (bytes, expected) in stream_strategy(),
        cuts in proptest::collection::vec(any::<prop::sample::Index>(), 0..16),
    ) {
        let mut positions: Vec<usize> = cuts.iter().map(|cut| cut.index(bytes.len() + 1)).collect();
        positions.sort_unstable();
        let mut chunks = Vec::new();
        let mut start = 0;
        for position in positions {
            chunks.push(&bytes[start..position]);
            start = position;
        }
        chunks.push(&bytes[start..]);

        let events = parse_all(&chunks).unwrap();

        prop_assert_eq!(events, expected);
    }

    #[test]
    fn arbitrary_bytes_never_panic(chunks in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..64), 0..16)) {
        let mut parser = SseParser::new();
        for chunk in &chunks {
            if parser.push(chunk).is_err() {
                break;
            }
        }
    }
}
