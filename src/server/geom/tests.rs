use super::*;

fn rect(x: u16, y: u16, cols: u16, rows: u16) -> Rect {
    Rect { x, y, cols, rows }
}

fn split(dir: Dir, first_size: u16) -> Layout {
    Layout::Split {
        dir,
        first: Box::new(Layout::Leaf(1)),
        second: Box::new(Layout::Leaf(2)),
        first_size,
    }
}

#[test]
fn a_framed_tab_keeps_the_frame_out_of_every_pane() {
    let geometry = Geometry::of(&split(Dir::Columns, 49), rect(0, 0, 100, 23), 2);
    assert_eq!(geometry.inner, rect(1, 1, 98, 21));
    assert_eq!(
        geometry.panes,
        vec![(1, rect(1, 1, 49, 21)), (2, rect(51, 1, 48, 21))]
    );
    assert_eq!(geometry.gutters.len(), 1);
    assert_eq!(geometry.gutters[0].rect, rect(50, 1, 1, 21));
    for (_, pane) in &geometry.panes {
        assert!(!geometry.on_frame(pane.x, pane.y));
    }
    assert!(geometry.on_frame(0, 5));
    assert!(geometry.on_frame(99, 5));
}

#[test]
fn a_single_pane_uses_the_whole_area() {
    let geometry = Geometry::of(&Layout::Leaf(1), rect(0, 0, 100, 23), 1);
    assert_eq!(geometry.panes, vec![(1, rect(0, 0, 100, 23))]);
    assert!(geometry.gutters.is_empty());
    assert!(!geometry.on_frame(0, 0));
    assert!(geometry.frame_cells().is_empty());
}

#[test]
fn zooming_ignores_the_split() {
    let geometry = Geometry::zoomed(2, rect(0, 0, 100, 23));
    assert_eq!(geometry.panes, vec![(2, rect(0, 0, 100, 23))]);
    assert!(geometry.gutters.is_empty());
}

#[test]
fn a_float_takes_two_thirds_and_centres() {
    let geometry = Geometry::of(&Layout::Leaf(1), rect(0, 0, 100, 23), 1).with_float(2);
    let float = geometry.float.expect("a floating pane");
    assert_eq!(float.frame, rect(17, 4, 66, 15));
    assert_eq!(float.inner, rect(18, 5, 64, 13));
    assert_eq!(
        geometry.visible(),
        vec![(1, rect(0, 0, 100, 23)), (2, rect(18, 5, 64, 13))]
    );
    assert!(geometry.on_float_frame(17, 4));
    assert!(!geometry.on_float_frame(18, 5));
    assert!(geometry.on_float_frame(82, 18));
    assert!(geometry.float_frame().contains(&(17, 4)));
    assert!(geometry.float_frame().contains(&(82, 18)));
    assert_eq!(geometry.float_frame().len(), 2 * (66 + 15) - 4);
}

#[test]
fn the_float_answers_a_hit_test_before_the_tiled_panes() {
    let geometry = Geometry::of(&split(Dir::Columns, 49), rect(0, 0, 100, 23), 2).with_float(3);
    assert_eq!(geometry.pane_at(50, 10).map(|(id, _)| id), Some(3));
    assert_eq!(geometry.pane_at(55, 1).map(|(id, _)| id), Some(2));
    assert_eq!(geometry.pane_at(5, 10).map(|(id, _)| id), Some(1));
    assert_eq!(geometry.float_cell(17, 10), Some((3, 0, 5)));
    assert_eq!(geometry.float_cell(19, 6), Some((3, 1, 1)));
    assert_eq!(geometry.float_cell(50, 1), None);
    assert!(geometry.under_float(17, 4));
    assert!(!geometry.under_float(16, 4));
}

#[test]
fn a_small_screen_shrinks_the_float_to_fit() {
    let geometry = Geometry::of(&Layout::Leaf(1), rect(0, 0, 20, 10), 1).with_float(2);
    let float = geometry.float.expect("a floating pane");
    assert_eq!(float.frame, rect(1, 1, 18, 7));
    assert_eq!(float.inner, rect(2, 2, 16, 5));
}

#[test]
fn a_tiny_screen_keeps_a_pty_inside_the_float() {
    let geometry = Geometry::of(&Layout::Leaf(1), rect(0, 0, 4, 4), 1).with_float(2);
    let float = geometry.float.expect("a floating pane");
    assert_eq!(float.frame, rect(1, 1, 2, 2));
    assert_eq!(float.inner, rect(2, 2, 0, 0));
}

#[test]
fn a_gutter_cell_resolves_to_its_split() {
    let geometry = Geometry::of(&split(Dir::Columns, 49), rect(0, 0, 100, 23), 2);
    let gutter = geometry
        .gutter_at(50, 10)
        .expect("gutter under the pointer");
    assert_eq!(gutter.path, Vec::<bool>::new());
    assert_eq!(gutter.area, rect(1, 1, 98, 21));
    assert!(geometry.gutter_at(51, 10).is_none());
}

#[test]
fn a_nested_gutter_carries_its_path() {
    let mut layout = split(Dir::Columns, 49);
    layout.split(2, 3, Dir::Rows, 10);
    let geometry = Geometry::of(&layout, rect(0, 0, 100, 23), 3);
    let gutter = geometry.gutter_at(60, 11).expect("nested gutter");
    assert_eq!(gutter.path, vec![true]);
    assert_eq!(gutter.dir, Dir::Rows);
}

#[test]
fn a_frame_cell_belongs_to_the_pane_it_touches() {
    let geometry = Geometry::of(&split(Dir::Columns, 49), rect(0, 0, 100, 23), 2);
    let touching: Vec<u64> = geometry
        .panes_next_to(0, 5)
        .iter()
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(touching, vec![1]);
    let touching: Vec<u64> = geometry
        .panes_next_to(99, 5)
        .iter()
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(touching, vec![2]);
    let around = geometry.frame_around(1);
    assert!(around.contains(&(0, 5)));
    assert!(around.contains(&(5, 0)));
    assert!(!around.contains(&(99, 5)));
}
