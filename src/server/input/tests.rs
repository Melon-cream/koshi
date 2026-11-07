use super::*;

fn run(input: &mut InputState, bytes: &[u8], leader: Leader) -> (Vec<u8>, Vec<Command>) {
    let mut pane = Vec::new();
    let mut out = Vec::new();
    input.feed(bytes, leader, false, &mut pane, &mut out);
    (pane, out)
}

#[test]
fn ctrl_leaders_reach_every_group() {
    let cases: &[(u8, LeaderKind)] = &[
        (0x10, LeaderKind::Pane),
        (0x14, LeaderKind::Tab),
        (0x0f, LeaderKind::Session),
        (0x13, LeaderKind::Scroll),
    ];
    for (byte, kind) in cases {
        let mut input = InputState::new();
        let (pane, out) = run(&mut input, &[*byte], Leader::Ctrl);
        assert_eq!(input, InputState::Leader(*kind));
        assert!(pane.is_empty());
        assert!(out.is_empty());
    }
}

#[test]
fn plain_letters_are_not_leaders() {
    let mut input = InputState::new();
    let (pane, out) = run(&mut input, b"pq", Leader::Ctrl);
    assert_eq!(input, InputState::Normal);
    assert_eq!(pane, b"pq");
    assert!(out.is_empty());
}

#[test]
fn pane_group_commands() {
    let cases: &[(&[u8], Command)] = &[
        (b"\x10p", Command::FocusNext),
        (b"\x10;", Command::FocusLast),
        (b"\x10n", Command::Split(Dir::Columns)),
        (b"\x10r", Command::Split(Dir::Columns)),
        (b"\x10d", Command::Split(Dir::Rows)),
        (b"\x10x", Command::ClosePane),
        (b"\x10f", Command::ToggleFloat),
        (b"\x10z", Command::ToggleZoom),
    ];
    for (bytes, expected) in cases {
        let mut input = InputState::new();
        let (pane, out) = run(&mut input, bytes, Leader::Ctrl);
        assert_eq!(input, InputState::Normal);
        assert!(pane.is_empty());
        assert_eq!(out, vec![expected.clone()]);
    }
}

#[test]
fn tab_and_session_commands() {
    let cases: &[(&[u8], Command)] = &[
        (b"\x14n", Command::NewTab),
        (b"\x14x", Command::CloseTab),
        (b"\x14r", Command::BeginTabRename),
        (b"\x14\t", Command::LastTab),
        (b"\x145", Command::GotoTab(4)),
        (b"\x0fw", Command::OpenWorkspaces),
        (b"\x0fr", Command::RenameWorkspace),
        (b"\x0fd", Command::Detach),
    ];
    for (bytes, expected) in cases {
        let mut input = InputState::new();
        let (pane, out) = run(&mut input, bytes, Leader::Ctrl);
        assert!(pane.is_empty());
        assert_eq!(out, vec![expected.clone()]);
    }
}

#[test]
fn scroll_commands() {
    let cases: &[(&[u8], Command)] = &[
        (b"\x13j", Command::ScrollLines(-1)),
        (b"\x13k", Command::ScrollLines(1)),
        (b"\x13\x06", Command::ScrollPages(-1)),
        (b"\x13\x02", Command::ScrollPages(1)),
        (b"\x13d", Command::ScrollHalfPages(-1)),
        (b"\x13u", Command::ScrollHalfPages(1)),
        (b"\x13\x03", Command::ScrollBottom),
    ];
    for (bytes, expected) in cases {
        let mut input = InputState::new();
        let (_, out) = run(&mut input, bytes, Leader::Ctrl);
        assert_eq!(out, vec![expected.clone()]);
    }
}

#[test]
fn leader_keys_focus_and_switch_tabs() {
    let cases: &[(u8, KeyCode, Command)] = &[
        (0x10, KeyCode::Left, Command::Focus(Side::Left)),
        (0x10, KeyCode::Up, Command::Focus(Side::Up)),
        (0x10, KeyCode::Down, Command::Focus(Side::Down)),
        (0x10, KeyCode::Right, Command::Focus(Side::Right)),
        (0x14, KeyCode::Left, Command::PrevTab),
        (0x14, KeyCode::Right, Command::NextTab),
        (0x13, KeyCode::Up, Command::ScrollLines(1)),
        (0x13, KeyCode::Down, Command::ScrollLines(-1)),
    ];
    for (leader, key, expected) in cases {
        let mut input = InputState::new();
        let (pane, mut out) = run(&mut input, &[*leader], Leader::Ctrl);
        assert!(pane.is_empty() && out.is_empty());
        let mut keys = Vec::new();
        input.feed_key(*key, false, &mut keys, &mut out);
        let expected_state = if *leader == 0x13 {
            InputState::Leader(LeaderKind::Scroll)
        } else {
            InputState::Normal
        };
        assert_eq!(input, expected_state);
        assert!(keys.is_empty(), "a leader key never reaches the pane");
        assert_eq!(
            out,
            vec![expected.clone()],
            "leader {leader:#04x} key {key:?}"
        );
    }
}

#[test]
fn the_scroll_group_stays_open_for_a_run_of_keys() {
    let mut input = InputState::new();
    let (pane, mut out) = run(&mut input, b"\x13k", Leader::Ctrl);
    assert!(pane.is_empty());
    assert_eq!(out, vec![Command::ScrollLines(1)]);
    assert_eq!(input, InputState::Leader(LeaderKind::Scroll));

    let mut keys = Vec::new();
    input.feed_key(KeyCode::Down, false, &mut keys, &mut out);
    assert_eq!(out, vec![Command::ScrollLines(1), Command::ScrollLines(-1)]);
    assert_eq!(input, InputState::Leader(LeaderKind::Scroll));

    // Esc leaves the group.
    let (pane, _) = run(&mut input, b"\x1b", Leader::Ctrl);
    assert!(pane.is_empty());
    assert_eq!(input, InputState::Normal);
}

#[test]
fn letters_are_not_bound_in_the_leader_groups() {
    for leader in [0x10u8, 0x14] {
        // The letters that used to move the focus or the tabs are unbound.
        for letter in *b"hjkl" {
            let mut input = InputState::new();
            let (pane, out) = run(&mut input, &[leader], Leader::Ctrl);
            assert!(pane.is_empty() && out.is_empty());
            let (pane, out) = run(&mut input, &[letter], Leader::Ctrl);
            assert_eq!(input, InputState::Normal);
            assert!(pane.is_empty(), "letter {letter:?} must not reach the pane");
            assert!(out.is_empty(), "letter {letter:?} must not run {out:?}");
        }
    }
}

#[test]
fn keys_outside_a_leader_reach_the_pane() {
    let mut input = InputState::new();
    let mut keys = Vec::new();
    let mut out = Vec::new();
    input.feed_key(KeyCode::Up, false, &mut keys, &mut out);
    input.feed_key(KeyCode::PageDown, false, &mut keys, &mut out);
    assert_eq!(input, InputState::Normal);
    assert_eq!(keys, vec![KeyCode::Up, KeyCode::PageDown]);
    assert!(out.is_empty());
}

#[test]
fn an_unbound_leader_key_is_dropped_with_the_leader() {
    let mut input = InputState::new();
    run(&mut input, &[0x10], Leader::Ctrl);
    let mut keys = Vec::new();
    let mut out = Vec::new();
    input.feed_key(KeyCode::PageUp, false, &mut keys, &mut out);
    assert_eq!(input, InputState::Normal);
    assert!(keys.is_empty());
    assert!(out.is_empty());
}

#[test]
fn cancellation_and_unknown_keys_return_to_normal() {
    for cancel in [&b"\x10\x1b"[..], b"\x10\x10", b"\x10q", b"\x14q"] {
        let mut input = InputState::new();
        let (pane, out) = run(&mut input, cancel, Leader::Ctrl);
        assert_eq!(input, InputState::Normal, "bytes {cancel:?}");
        assert!(pane.is_empty(), "bytes {cancel:?}");
        assert!(out.is_empty(), "bytes {cancel:?}");
    }
}

#[test]
fn alt_leaders_use_the_next_letter() {
    let mut input = InputState::new();
    let (pane, out) = run(&mut input, b"\x1bp", Leader::Alt);
    assert_eq!(input, InputState::Leader(LeaderKind::Pane));
    assert!(pane.is_empty() && out.is_empty());

    let (pane, out) = run(&mut input, b"r", Leader::Alt);
    assert_eq!(input, InputState::Normal);
    assert!(pane.is_empty());
    assert_eq!(out, vec![Command::Split(Dir::Columns)]);
}

#[test]
fn alt_leaders_leave_ctrl_and_a_lone_esc_alone() {
    let mut input = InputState::new();
    let (pane, out) = run(&mut input, b"\x10", Leader::Alt);
    assert_eq!(input, InputState::Normal);
    assert_eq!(pane, b"\x10");
    assert!(out.is_empty());

    let (pane, out) = run(&mut input, b"\x1bq", Leader::Alt);
    assert_eq!(input, InputState::Normal);
    assert_eq!(pane, b"\x1bq");
    assert!(out.is_empty(), "q belongs to no group: {out:?}");

    // A trailing Esc without a letter stays pane input.
    let (pane, _) = run(&mut input, b"\x1b", Leader::Alt);
    assert_eq!(pane, b"\x1b");
}

#[test]
fn the_list_opens_the_settings_screen() {
    let mut input = InputState::Overlay;
    let (pane, out) = run(&mut input, b"c", Leader::Ctrl);
    assert!(pane.is_empty());
    assert_eq!(out, vec![Command::OverlaySettings]);
    assert_eq!(input, InputState::Overlay);
}

fn settings() -> InputState {
    InputState::Settings {
        workspace: 3,
        selected: 0,
        check: None,
    }
}

#[test]
fn the_settings_screen_takes_arrows_enter_and_esc() {
    let cases: &[(&[u8], Command)] = &[
        (b"\r", Command::SettingsEdit),
        (b"\n", Command::SettingsEdit),
        (b"\x1b", Command::CloseSettings),
    ];
    for (bytes, expected) in cases {
        let mut input = settings();
        let (_, out) = run(&mut input, bytes, Leader::Ctrl);
        assert_eq!(out, vec![expected.clone()], "bytes {bytes:?}");
    }

    let mut input = settings();
    let mut keys = Vec::new();
    let mut out = Vec::new();
    input.feed_key(KeyCode::Up, false, &mut keys, &mut out);
    input.feed_key(KeyCode::Down, false, &mut keys, &mut out);
    assert!(keys.is_empty());
    assert_eq!(out, vec![Command::SettingsUp, Command::SettingsDown]);
    assert_eq!(input, settings());
}

#[test]
fn the_alias_buffer_stops_at_the_limit() {
    let mut input = InputState::Text {
        target: TextTarget::SshAlias(1),
        buffer: String::new(),
    };
    let (_, out) = run(&mut input, &[b'x'; 300], Leader::Ctrl);
    assert!(out.is_empty());
    let InputState::Text { buffer, .. } = &input else {
        panic!("still editing the alias: {input:?}");
    };
    assert_eq!(buffer.len(), 255);
}

#[test]
fn an_empty_alias_commits_instead_of_cancelling() {
    let mut input = InputState::Text {
        target: TextTarget::SshAlias(4),
        buffer: String::new(),
    };
    let (_, out) = run(&mut input, b"\r", Leader::Ctrl);
    assert_eq!(
        out,
        vec![Command::CommitText(TextTarget::SshAlias(4), String::new())]
    );

    let mut input = InputState::Text {
        target: TextTarget::SshAlias(4),
        buffer: String::new(),
    };
    let (_, out) = run(&mut input, b"\x1b", Leader::Ctrl);
    assert_eq!(out, vec![Command::CancelText(TextTarget::SshAlias(4))]);
}
