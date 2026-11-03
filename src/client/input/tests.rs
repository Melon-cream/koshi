use super::*;

fn feed(parser: &mut InputParser, bytes: &[u8]) -> Vec<ClientInput> {
    let mut out = Vec::new();
    parser.feed(bytes, &mut out);
    out
}

fn only_key(out: Vec<ClientInput>) -> KeyCode {
    assert_eq!(out.len(), 1, "expected one event");
    match out.into_iter().next() {
        Some(ClientInput::Key(key)) => key,
        _ => panic!("expected a key"),
    }
}

#[test]
fn csi_and_ss3_arrows_become_keys() {
    let cases: &[(&[u8], KeyCode)] = &[
        (b"\x1b[A", KeyCode::Up),
        (b"\x1b[B", KeyCode::Down),
        (b"\x1b[C", KeyCode::Right),
        (b"\x1b[D", KeyCode::Left),
        (b"\x1bOA", KeyCode::Up),
        (b"\x1bOD", KeyCode::Left),
        (b"\x1b[H", KeyCode::Home),
        (b"\x1b[F", KeyCode::End),
        (b"\x1b[1~", KeyCode::Home),
        (b"\x1b[4~", KeyCode::End),
        (b"\x1b[5~", KeyCode::PageUp),
        (b"\x1b[6~", KeyCode::PageDown),
    ];
    for (bytes, expected) in cases {
        let mut parser = InputParser::new();
        assert_eq!(
            only_key(feed(&mut parser, bytes)),
            *expected,
            "bytes {bytes:?}"
        );
    }
}

#[test]
fn a_sequence_split_across_reads_is_still_one_key() {
    let mut parser = InputParser::new();
    assert!(feed(&mut parser, b"\x1b").is_empty());
    assert_eq!(only_key(feed(&mut parser, b"[B")), KeyCode::Down);

    let mut parser = InputParser::new();
    assert!(feed(&mut parser, b"\x1b[").is_empty());
    assert_eq!(only_key(feed(&mut parser, b"5~")), KeyCode::PageUp);

    let mut parser = InputParser::new();
    assert!(feed(&mut parser, b"\x1bO").is_empty());
    assert_eq!(only_key(feed(&mut parser, b"C")), KeyCode::Right);
}

#[test]
fn modified_and_unknown_sequences_stay_raw() {
    let mut parser = InputParser::new();
    assert!(matches!(
        feed(&mut parser, b"\x1b[1;5A").as_slice(),
        [ClientInput::Bytes(bytes)] if bytes == b"\x1b[1;5A"
    ));
    let mut parser = InputParser::new();
    assert!(matches!(
        feed(&mut parser, b"\x1bx").as_slice(),
        [ClientInput::Bytes(bytes)] if bytes == b"\x1bx"
    ));
}

#[test]
fn a_lone_esc_waits_for_the_idle_flush() {
    let mut parser = InputParser::new();
    assert!(feed(&mut parser, b"\x1b").is_empty());
    let mut out = Vec::new();
    parser.flush_idle(&mut out);
    assert!(matches!(out.as_slice(), [ClientInput::Bytes(bytes)] if bytes == b"\x1b"));

    let mut parser = InputParser::new();
    assert!(feed(&mut parser, b"\x1bO").is_empty());
    let mut out = Vec::new();
    parser.flush_idle(&mut out);
    assert!(matches!(out.as_slice(), [ClientInput::Bytes(bytes)] if bytes == b"\x1bO"));
}

#[test]
fn mouse_reports_are_still_parsed() {
    let mut parser = InputParser::new();
    let out = feed(&mut parser, b"\x1b[<0;5;3M");
    assert!(matches!(
        out.as_slice(),
        [ClientInput::Mouse(MouseInput {
            action: MouseAction::Press,
            button: MouseButton::Left,
            col: 4,
            row: 2,
            ..
        })]
    ));
}
