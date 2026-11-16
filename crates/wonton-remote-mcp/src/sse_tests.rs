use super::{EventError, MAX_EVENT_BYTES, MAX_LINE_BYTES, decode, encode};
use axum::body::Bytes;
use futures_util::{StreamExt, stream};
use sse_stream::Sse;
use std::{io, time::Duration};

async fn chunks(bytes: Vec<Bytes>) -> Vec<Result<Sse, EventError>> {
    decode(stream::iter(bytes.into_iter().map(Ok::<_, io::Error>)))
        .collect()
        .await
}

fn data_line(raw_length: usize) -> String {
    let line = format!("data: {}", "a".repeat(raw_length - 6));
    assert_eq!(line.len(), raw_length);
    line
}

#[tokio::test]
async fn exact_line_limit_and_one_byte_over_for_all_delimiters() {
    for delimiter in ["\n", "\r", "\r\n"] {
        for extra in [0, 1] {
            let line = data_line(MAX_LINE_BYTES + extra);
            let wire = format!("{line}{delimiter}{delimiter}");
            assert_eq!(wire.len(), MAX_LINE_BYTES + extra + delimiter.len() * 2);
            let result = chunks(vec![Bytes::from(wire)]).await;
            if extra == 0 {
                assert_eq!(
                    result,
                    vec![Ok(Sse::default().data("a".repeat(MAX_LINE_BYTES - 6)))]
                );
            } else {
                assert_eq!(result, vec![Err(EventError::TooLarge)]);
            }
        }
    }
}

#[tokio::test]
async fn fragmented_unterminated_line_fails_without_waiting_for_eof() {
    let first = Bytes::from(data_line(MAX_LINE_BYTES / 2));
    let second = Bytes::from("a".repeat(MAX_LINE_BYTES - first.len()));
    assert_eq!(first.len() + second.len(), MAX_LINE_BYTES);
    let input = stream::iter([first, second, Bytes::from_static(b"a")].map(Ok::<_, io::Error>))
        .chain(stream::pending());
    let events = decode(input);
    futures_util::pin_mut!(events);
    let result = tokio::time::timeout(Duration::from_secs(1), events.next())
        .await
        .expect("the guard must fail before the unfinished line ends");
    assert_eq!(result, Some(Err(EventError::TooLarge)));
    assert_eq!(events.next().await, None);
}

#[tokio::test]
async fn multiline_event_limit_is_exact_and_independent_of_line_limit() {
    for extra in [0, 1] {
        let first = data_line(MAX_EVENT_BYTES / 2);
        let second = data_line(MAX_EVENT_BYTES - first.len() + extra);
        assert!(first.len() < MAX_LINE_BYTES);
        assert!(second.len() < MAX_LINE_BYTES);
        assert_eq!(first.len() + second.len(), MAX_EVENT_BYTES + extra);
        let wire = format!("{first}\n{second}\n\n");
        let result = chunks(vec![Bytes::from(wire)]).await;
        if extra == 0 {
            assert_eq!(
                result,
                vec![Ok(Sse::default().data(format!(
                    "{}\n{}",
                    &first[6..],
                    &second[6..]
                )))]
            );
        } else {
            assert_eq!(result, vec![Err(EventError::TooLarge)]);
        }
    }
}

#[tokio::test]
async fn many_small_lines_cannot_bypass_event_limit() {
    // Each line is tiny; only the unfinished event is over the bound.
    let repeats = MAX_EVENT_BYTES / 8;
    let prefix = "data: aa\n".repeat(repeats);
    assert_eq!(repeats * "data: aa".len(), MAX_EVENT_BYTES);
    let input = stream::iter([
        Ok::<_, io::Error>(Bytes::from(prefix)),
        Ok(Bytes::from_static(b"d")),
    ])
    .chain(stream::pending());
    let events = decode(input);
    futures_util::pin_mut!(events);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), events.next())
            .await
            .expect("event bound must not require a blank line"),
        Some(Err(EventError::TooLarge))
    );
}

#[tokio::test]
async fn cumulative_stream_size_is_not_an_event_limit() {
    let expected = Sse::default().data("a".repeat(2048));
    let frame = encode(expected.clone());
    let count = 1024;
    assert!(frame.len() * count > MAX_EVENT_BYTES);
    let wire = frame.repeat(count);
    let together = chunks(vec![Bytes::from(wire)]).await;
    assert_eq!(together.len(), count);
    assert!(together.iter().all(|event| event.as_ref() == Ok(&expected)));
    let separate = chunks(vec![frame; count]).await;
    assert_eq!(separate, together);
}

#[tokio::test]
async fn bom_and_crlf_boundaries_work_with_byte_sized_chunks() {
    for prefix in ["\u{feff}", "\u{feff}\r\n"] {
        let wire = format!("{prefix}id: first\r\ndata: one\r\n\r\ndata: two\r\r");
        let bytes = wire.bytes().map(|byte| Bytes::from(vec![byte])).collect();
        assert_eq!(
            chunks(bytes).await,
            vec![
                Ok(Sse::default().id("first").data("one")),
                Ok(Sse::default().data("two")),
            ]
        );
    }
}

#[tokio::test]
async fn bom_bytes_are_included_in_exact_raw_limit() {
    for extra in [0, 1] {
        let line = data_line(MAX_LINE_BYTES - 3 + extra);
        let wire = format!("\u{feff}{line}\n\n");
        assert_eq!(wire.len() - 2, MAX_LINE_BYTES + extra);
        let result = chunks(vec![Bytes::from(wire)]).await;
        if extra == 0 {
            assert_eq!(result, vec![Ok(Sse::default().data(&line[6..]))]);
        } else {
            assert_eq!(result, vec![Err(EventError::TooLarge)]);
        }
    }
}

#[tokio::test]
async fn encoder_preserves_multiline_json_empty_lines_and_metadata() {
    for data in [
        "",
        "\n",
        "\n\n",
        "\nfirst\n\nlast\n",
        "{\n  \"jsonrpc\": \"2.0\",\n  \"id\": 1,\n  \"result\": {}\n}",
    ] {
        let event = Sse::default()
            .event("message")
            .id("cursor")
            .retry(1500)
            .data(data);
        let encoded = encode(event.clone());
        let wire = std::str::from_utf8(&encoded).unwrap();
        let data_lines: Vec<_> = wire
            .lines()
            .filter(|line| line.starts_with("data:"))
            .collect();
        assert_eq!(data_lines.len(), data.split('\n').count());
        for (line, original) in data_lines.iter().zip(data.split('\n')) {
            assert_eq!(*line, format!("data: {original}"));
        }
        let decoded = chunks(vec![encoded]).await;
        assert_eq!(decoded, vec![Ok(event)]);
        if data.starts_with('{') {
            let parsed: serde_json::Value =
                serde_json::from_str(decoded[0].as_ref().unwrap().data.as_deref().unwrap())
                    .unwrap();
            assert_eq!(parsed["id"], 1);
        }
    }
}

#[tokio::test]
async fn encoder_preserves_id_only_and_empty_control_events() {
    for event in [
        Sse::default().id("cursor"),
        Sse::default().id("").retry(0),
        Sse::default().id("start").data(""),
        Sse::default().data(" value with leading space"),
    ] {
        assert_eq!(chunks(vec![encode(event.clone())]).await, vec![Ok(event)]);
    }
}

#[tokio::test]
async fn oversized_event_does_not_hide_prior_event_or_poison_other_streams() {
    let mut wire = b"data: before\n\n".to_vec();
    wire.extend_from_slice(data_line(MAX_LINE_BYTES + 1).as_bytes());
    wire.extend_from_slice(b"\n\ndata: after\n\n");
    assert_eq!(
        chunks(vec![Bytes::from(wire)]).await,
        vec![Ok(Sse::default().data("before")), Err(EventError::TooLarge)]
    );
    assert_eq!(
        chunks(vec![Bytes::from_static(b"data: isolated\n\n")]).await,
        vec![Ok(Sse::default().data("isolated"))]
    );
}

#[tokio::test]
async fn transport_and_invalid_errors_are_terminal_and_redacted() {
    let secret = "https://example.invalid/private?token=secret";
    let transport = decode(stream::iter([
        Err(io::Error::other(secret)),
        Ok(Bytes::from_static(b"data: must not follow\n\n")),
    ]))
    .collect::<Vec<_>>()
    .await;
    assert_eq!(transport, vec![Err(EventError::Transport)]);
    let invalid = chunks(vec![Bytes::from(format!("{secret}\n\ndata: ignored\n\n"))]).await;
    assert_eq!(invalid, vec![Err(EventError::Invalid)]);
    for error in [
        EventError::TooLarge,
        EventError::Transport,
        EventError::Invalid,
    ] {
        assert!(!error.to_string().contains(secret));
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[tokio::test]
async fn empty_transport_chunks_do_not_break_crlf_or_bom_parsing() {
    let bytes = vec![
        Bytes::new(),
        Bytes::from_static(b"\xef"),
        Bytes::new(),
        Bytes::from_static(b"\xbb\xbfdata: value\r"),
        Bytes::new(),
        Bytes::from_static(b"\n\r"),
        Bytes::new(),
        Bytes::from_static(b"\n"),
    ];
    assert_eq!(chunks(bytes).await, vec![Ok(Sse::default().data("value"))]);
}
