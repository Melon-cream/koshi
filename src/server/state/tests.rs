use super::*;

fn sample() -> SavedState {
    SavedState {
        active: 0,
        workspaces: vec![SavedWorkspace {
            id: 1,
            name: "main".to_string(),
            ssh_alias: Some("myhost".to_string()),
            record: false,
            active: 0,
            tabs: vec![SavedTab {
                title: "shell".to_string(),
                layout: Layout::Split {
                    dir: crate::server::layout::Dir::Columns,
                    first: Box::new(Layout::Leaf(1)),
                    second: Box::new(Layout::Leaf(2)),
                    first_size: 20,
                },
                focus: 2,
                zoomed: false,
                panes: vec![
                    SavedPane {
                        id: 1,
                        cwd: Some("/tmp".to_string()),
                        cols: 80,
                        rows: 24,
                        lines: vec!["hello".to_string(), String::new()],
                    },
                    SavedPane {
                        id: 2,
                        cwd: None,
                        cols: 40,
                        rows: 12,
                        lines: vec![],
                    },
                ],
            }],
        }],
    }
}

/// A format-1 file: two workspaces, framed with version 1 and its checksum.
fn encode_v1(state: &v1::StateV1) -> Vec<u8> {
    let payload = postcard::to_stdvec(state).unwrap();
    frame(1, &payload)
}

fn sample_v1() -> v1::StateV1 {
    v1::StateV1 {
        active: 1,
        workspaces: vec![
            v1::WorkspaceV1 {
                name: "main".to_string(),
                active: 0,
                tabs: vec![v1::TabV1 {
                    title: "logs".to_string(),
                    layout: Layout::Leaf(1),
                    focus: 1,
                    zoomed: false,
                    panes: vec![v1::PaneV1 {
                        id: 1,
                        cwd: Some("/tmp".to_string()),
                        cols: 80,
                        rows: 24,
                        lines: vec!["old".to_string()],
                    }],
                }],
            },
            v1::WorkspaceV1 {
                name: "ws2".to_string(),
                active: 0,
                tabs: vec![v1::TabV1 {
                    title: "build".to_string(),
                    layout: Layout::Leaf(2),
                    focus: 2,
                    zoomed: false,
                    panes: vec![v1::PaneV1 {
                        id: 2,
                        cwd: None,
                        cols: 80,
                        rows: 24,
                        lines: vec![],
                    }],
                }],
            },
        ],
    }
}

#[test]
fn roundtrip() {
    let state = sample();
    let bytes = encode(&state).unwrap();
    assert_eq!(&bytes[..6], MAGIC);
    assert_eq!(decode(&bytes).unwrap(), state);
}

#[test]
fn the_header_carries_the_current_format() {
    let bytes = encode(&sample()).unwrap();
    assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]), 2);
}

#[test]
fn a_format_1_file_is_read_as_format_2() {
    let bytes = encode_v1(&sample_v1());
    let state = decode(&bytes).unwrap();
    assert_eq!(state.active, 1);
    assert_eq!(state.workspaces.len(), 2);
    for (index, workspace) in state.workspaces.iter().enumerate() {
        assert_eq!(workspace.id, index as u64 + 1);
        assert_eq!(workspace.ssh_alias, None);
        assert!(!workspace.record);
    }
    assert_eq!(state.workspaces[0].name, "main");
    assert_eq!(state.workspaces[0].tabs[0].title, "logs");
    assert_eq!(state.workspaces[1].name, "ws2");
    assert_eq!(state.workspaces[1].tabs[0].title, "build");
    assert_eq!(state.workspaces[0].tabs[0].panes[0].lines, vec!["old"]);
}

#[test]
fn rejects_damaged_files() {
    let bytes = encode(&sample()).unwrap();
    assert!(decode(b"KOSHI").is_err());
    assert!(
        decode(b"NOTKOSHI\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00")
            .is_err()
    );

    let mut wrong_version = bytes.clone();
    wrong_version[6] = 9;
    assert!(decode(&wrong_version).is_err());

    let mut flipped = bytes.clone();
    let last = flipped.len() - 1;
    flipped[last] ^= 0xff;
    assert!(decode(&flipped).is_err());

    let truncated = &bytes[..bytes.len() - 1];
    assert!(decode(truncated).is_err());
}

#[test]
fn a_damaged_format_1_file_is_rejected() {
    let mut bytes = encode_v1(&sample_v1());
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    assert!(decode(&bytes).is_err());
}

#[test]
fn alias_limits() {
    assert!(!alias_is_valid("bad\u{7}alias"));
    assert!(!alias_is_valid("bad\nalias"));
    assert!(!alias_is_valid(&"x".repeat(256)));
    assert!(alias_is_valid(&"x".repeat(255)));
    assert!(alias_is_valid("my host; touch /tmp/x $HOME"));
}

#[test]
fn an_invalid_alias_is_dropped_and_counted() {
    let mut state = sample();
    state.workspaces[0].ssh_alias = Some("bad\u{1}alias".to_string());
    let line = take_invalid_aliases(&mut state);
    assert_eq!(state.workspaces[0].ssh_alias, None);
    assert!(line.contains("dropped an invalid ssh alias"), "{line}");
    assert!(line.contains('1'), "{line}");
}

#[test]
fn trims_to_the_caps() {
    let mut state = sample();
    state.workspaces[0].tabs[0].panes[0].lines =
        (0..2500).map(|index| format!("line {index}")).collect();
    state.trim_history();
    let lines = &state.workspaces[0].tabs[0].panes[0].lines;
    assert_eq!(lines.len(), MAX_PANE_LINES);
    assert_eq!(lines[0], "line 500");
    assert_eq!(lines[MAX_PANE_LINES - 1], "line 2499");
}

#[test]
fn drops_the_oldest_lines_over_the_byte_budget() {
    let mut state = sample();
    let long = "x".repeat(1024);
    state.workspaces[0].tabs[0].panes[0].lines = vec![long.clone(); 600];
    state.trim_history();
    let kept: usize = state
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.tabs)
        .flat_map(|tab| &tab.panes)
        .map(|pane| pane.lines.iter().map(|line| line.len() + 1).sum::<usize>())
        .sum();
    assert!(kept <= MAX_HISTORY_BYTES, "kept {kept}");
    assert!(!state.workspaces[0].tabs[0].panes[0].lines.is_empty());
}

/// A workspace whose shape matches what `restore` accepts.
fn saved_workspace(id: u64, name: &str, alias: Option<&str>) -> SavedWorkspace {
    SavedWorkspace {
        id,
        name: name.to_string(),
        ssh_alias: alias.map(str::to_string),
        record: false,
        active: 0,
        tabs: vec![SavedTab {
            title: "shell".to_string(),
            layout: Layout::Leaf(1),
            focus: 1,
            zoomed: false,
            panes: vec![SavedPane {
                id: 1,
                cwd: None,
                cols: 80,
                rows: 24,
                lines: Vec::new(),
            }],
        }],
    }
}

fn session() -> Session {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let record_root = std::env::temp_dir().join("koshi-test-record");
    Session::new(
        100,
        1,
        tx,
        crate::config::Leader::Ctrl,
        crate::server::session::RecordSettings {
            root: record_root,
            retain_days: 0,
        },
        80,
        24,
    )
    .expect("the test needs a pty")
}

#[test]
fn restore_continues_the_workspace_ids() {
    let saved = SavedState {
        active: 1,
        workspaces: vec![
            saved_workspace(7, "main", None),
            saved_workspace(9, "ws2", Some("myhost")),
        ],
    };
    let mut session = session();
    assert!(session.restore(saved));
    assert_eq!(session.next_workspace, 10);
    assert_eq!(session.workspaces[1].id, 9);
    assert_eq!(session.workspaces[1].ssh_alias.as_deref(), Some("myhost"));
}
