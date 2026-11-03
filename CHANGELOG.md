# Changelog

## [0.1.1] - 2026-11-03

### Added

- `KOSHI_PANE` is set in every pane shell, so a startup file can tell it already runs
  inside koshi and skip a nested attach

### Fixed

- A missing `XDG_RUNTIME_DIR`, as on WSL without systemd, falls back to the temporary
  directory instead of failing to create `/run/user/<uid>`
- A server that fails to start now prints why, and a connection timeout points at
  `koshi server`

## [0.1.0] - 2026-11-03

### Added

- Panes, tabs, and named workspaces
- Four leader groups and Alt+arrow focus movement; the scroll group stays open for a run
  of keys
- Click to focus, wheel for history, drag to select and copy with OSC 52
- Pane zoom and a floating pane
- Dragging a split border to resize panes
- Detach and reattach, with the layout and history restored after a restart
- Workspace list
