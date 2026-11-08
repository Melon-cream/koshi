# Changelog

## [0.2.0] - 2026-11-08

### Added

- SSH workspaces: a workspace keeps one alias from `~/.ssh/config`, and every new pane starts
  `ssh <alias>` as a connection of its own, so reuse is left to ControlMaster in that config.
  The workspace list shows `ssh:<alias>`, `c` opens the settings screen, and confirming an
  alias runs `ssh -G` once to report whether it resolves
- Pane output recording, per workspace and off by default. `record:` on the settings screen
  starts or stops the panes that are already running. Settled lines are written to
  `<root>/<YYYYMMDD>/<workspace>-<id>/<pane>-<HHMMSS>.log`, with a `.times` sidecar holding one
  ISO 8601 local timestamp per line. Colours, cursor control, and the alternate screen are not
  recorded
- Weekly archive: a finished week is packed into `<root>/<YYYYMM>/<start>-<end>.tar.gz`, and
  `record_retain_days` deletes archives past their age
- `koshi record show [workspace] [date]` browses workspaces, dates, and logs, and Tab switches
  between Per-pane (the default) and Chronological
- `record_root` and `record_retain_days` in `config.toml`
- `ROADMAP.md`

### Changed

- The saved state moves to format 2: a workspace gained `id`, `ssh_alias`, and `record`. A
  format-1 file is migrated when it is read
- An SSH workspace keeps its definition with no panes and is restored after a restart

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
