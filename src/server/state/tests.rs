use super::*;

fn sample() -> SavedState {
    SavedState {
        active: 0,
        workspaces: vec![SavedWorkspace {
            name: "main".to_string(),
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

#[test]
fn roundtrip() {
    let state = sample();
    let bytes = encode(&state).unwrap();
    assert_eq!(&bytes[..6], MAGIC);
    assert_eq!(decode(&bytes).unwrap(), state);
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
