//! The workspace list and the workspace settings screen.

use crate::protocol::Overlay;
use crate::server::input::{AliasCheck, InputState, TextTarget};
use crate::server::session::Client;
use crate::server::tab::Workspace;

/// `None` unless the client is in the workspace list, its settings screen, or
/// renaming in the list.
pub(in crate::server) fn build(
    entries: &[(String, usize, usize, Option<String>)],
    active: usize,
    client: &Client,
    workspaces: &[Workspace],
) -> Option<Overlay> {
    match &client.input {
        InputState::Settings {
            workspace,
            selected,
            check,
        } => {
            let target = workspaces.iter().find(|entry| entry.id == *workspace)?;
            Some(settings(target, *selected, check.as_ref(), None))
        }
        InputState::Text {
            target: TextTarget::SshAlias(workspace),
            buffer,
        } => {
            let target = workspaces.iter().find(|entry| entry.id == *workspace)?;
            Some(settings(target, 0, None, Some(buffer)))
        }
        InputState::Overlay
        | InputState::Text {
            target: TextTarget::Workspace(_),
            ..
        } => Some(list(entries, active, client)),
        _ => None,
    }
}

fn list(
    entries: &[(String, usize, usize, Option<String>)],
    active: usize,
    client: &Client,
) -> Overlay {
    let items = entries
        .iter()
        .enumerate()
        .map(|(index, (name, tabs, panes, alias))| {
            let ssh = match alias {
                Some(alias) => format!("ssh:{alias}"),
                None => String::new(),
            };
            format!(
                "{} {:<16} {:<14} {} tabs, {} panes{}",
                if index == client.overlay_selected {
                    ">"
                } else {
                    " "
                },
                name,
                ssh,
                tabs,
                panes,
                if index == active { " (current)" } else { "" }
            )
        })
        .collect();
    let prompt = match &client.input {
        InputState::Text {
            target: TextTarget::Workspace(_),
            buffer,
        } => Some(format!("rename: {buffer}_")),
        _ => None,
    };
    Overlay {
        title: "Workspaces".to_string(),
        items,
        selected: client.overlay_selected,
        prompt,
        footer: "n new  r rename  d delete  c settings  Enter switch  Esc close".to_string(),
    }
}

fn settings(
    workspace: &Workspace,
    selected: usize,
    check: Option<&AliasCheck>,
    buffer: Option<&String>,
) -> Overlay {
    let alias = workspace.ssh_alias.as_deref().unwrap_or("(none)");
    let mut footer = Vec::new();
    if let Some(check) = check {
        footer.push(format!(
            "ssh -G {}: {}",
            check.alias,
            if check.ok { "ok" } else { "failed" }
        ));
    }
    footer.push("Enter edit".to_string());
    footer.push("Esc back".to_string());
    Overlay {
        title: "Settings".to_string(),
        items: vec![
            format!("ssh alias: {alias}"),
            format!("record: {}", if workspace.record { "on" } else { "off" }),
        ],
        selected,
        prompt: buffer.map(|text| format!("alias: {text}_")),
        footer: footer.join("  "),
    }
}
