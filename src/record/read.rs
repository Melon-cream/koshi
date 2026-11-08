//! Reading recordings back for the viewer: which workspaces exist, which days
//! they have, and the lines of one day.
//!
//! Both the uncompressed directories and the weekly archives are sources. An
//! archive is read with `tar` straight to stdout, never extracted, because it
//! shares a directory with the recordings the server is still writing.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::{Date, parse_workspace_dir};

#[derive(Clone)]
pub(crate) struct WorkspaceRecord {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) days: usize,
}

pub(crate) struct LogLine {
    pub(crate) pane: u64,
    pub(crate) time: Option<String>,
    pub(crate) text: String,
}

pub(crate) struct DayLog {
    pub(crate) lines: Vec<LogLine>,
    pub(crate) warning: bool,
}

/// Every workspace that has a record, lowest id first.
pub(crate) fn workspaces(root: &Path) -> Vec<WorkspaceRecord> {
    let mut seen: BTreeMap<u64, (String, Date, BTreeSet<Date>)> = BTreeMap::new();
    for (date, name, id) in uncompressed(root) {
        let entry = seen
            .entry(id)
            .or_insert_with(|| (name.clone(), date, BTreeSet::new()));
        if date.days() > entry.1.days() {
            entry.0 = name;
            entry.1 = date;
        }
        entry.2.insert(date);
    }
    for (date, name, id) in archived(root) {
        let entry = seen
            .entry(id)
            .or_insert_with(|| (name.clone(), date, BTreeSet::new()));
        if date.days() > entry.1.days() {
            entry.0 = name;
            entry.1 = date;
        }
        entry.2.insert(date);
    }
    seen.into_iter()
        .map(|(id, (name, _, days))| WorkspaceRecord {
            id,
            name,
            days: days.len(),
        })
        .collect()
}

/// The days one workspace has a record for, newest first.
pub(crate) fn days(root: &Path, workspace_id: u64) -> Vec<Date> {
    let mut days = BTreeSet::new();
    for (date, _, id) in uncompressed(root) {
        if id == workspace_id {
            days.insert(date);
        }
    }
    for (date, _, id) in archived(root) {
        if id == workspace_id {
            days.insert(date);
        }
    }
    days.into_iter().rev().collect()
}

/// The lines of one workspace on one day.
pub(crate) fn load(root: &Path, workspace_id: u64, date: Date) -> io::Result<DayLog> {
    let mut lines = Vec::new();
    let mut warning = false;

    if let Ok(entries) = fs::read_dir(root.join(date.compact())) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some((_, id)) = parse_workspace_dir(&name) else {
                continue;
            };
            if id != workspace_id {
                continue;
            }
            read_directory(&entry.path(), &mut lines, &mut warning)?;
        }
    }

    for archive in covering(root, date) {
        let members = match members(&archive) {
            Ok(members) => members,
            Err(_) => continue,
        };
        let prefix = format!("{}/", date.compact());
        let mut bodies: BTreeMap<u64, (String, Option<String>)> = BTreeMap::new();
        for member in &members {
            let Some(rest) = member.strip_prefix(&prefix) else {
                continue;
            };
            let Some((workspace, file)) = rest.split_once('/') else {
                continue;
            };
            let Some((_, id)) = parse_workspace_dir(workspace) else {
                continue;
            };
            if id != workspace_id {
                continue;
            }
            let Some(stem) = file.strip_suffix(".log") else {
                continue;
            };
            let body = read_member(&archive, member)?;
            let times = read_member(&archive, &format!("{prefix}{workspace}/{stem}.times")).ok();
            push_paired(&mut bodies, stem, body, times);
        }
        for (pane, (body, times)) in bodies {
            push_body(&mut lines, pane, body, times.as_deref(), &mut warning);
        }
    }

    lines.sort_by(|left, right| {
        left.time
            .cmp(&right.time)
            .then_with(|| left.pane.cmp(&right.pane))
    });
    Ok(DayLog { lines, warning })
}

/// `(date, workspace name, workspace id)` for every uncompressed directory.
fn uncompressed(root: &Path) -> Vec<(Date, String, u64)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(date) = Date::parse(&name) else {
            continue;
        };
        if !entry.path().is_dir() {
            continue;
        }
        let Ok(workspaces) = fs::read_dir(entry.path()) else {
            continue;
        };
        for workspace in workspaces.flatten() {
            let Some((name, id)) = parse_workspace_dir(&workspace.file_name().to_string_lossy())
            else {
                continue;
            };
            out.push((date, name, id));
        }
    }
    out
}

/// `(date, workspace name, workspace id)` for every member of every archive.
fn archived(root: &Path) -> Vec<(Date, String, u64)> {
    let mut out = Vec::new();
    for archive in archives(root) {
        let Ok(members) = members(&archive) else {
            continue;
        };
        for member in members {
            let mut parts = member.split('/');
            let (Some(day), Some(workspace)) = (parts.next(), parts.next()) else {
                continue;
            };
            let Some(date) = Date::parse(day) else {
                continue;
            };
            let Some((name, id)) = parse_workspace_dir(workspace) else {
                continue;
            };
            out.push((date, name, id));
        }
    }
    out
}

/// Every archive under the root, whatever its week.
fn archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(months) = fs::read_dir(root) else {
        return out;
    };
    for month in months.flatten() {
        let Ok(files) = fs::read_dir(month.path()) else {
            continue;
        };
        for file in files.flatten() {
            let name = file.file_name().to_string_lossy().into_owned();
            if name.ends_with(".tar.gz") {
                out.push(file.path());
            }
        }
    }
    out.sort();
    out
}

/// The archives whose week contains `date`.
fn covering(root: &Path, date: Date) -> Vec<PathBuf> {
    archives(root)
        .into_iter()
        .filter(|archive| {
            week_of(archive).is_some_and(|(start, end)| {
                start.days() <= date.days() && date.days() <= end.days()
            })
        })
        .collect()
}

/// The Monday and Sunday an archive file name carries.
fn week_of(archive: &Path) -> Option<(Date, Date)> {
    let name = archive.file_name()?.to_string_lossy();
    let stem = name.strip_suffix(".tar.gz")?;
    let (start, end) = stem.split_once('-')?;
    Some((Date::parse(start)?, Date::parse(end)?))
}

fn members(archive: &Path) -> io::Result<Vec<String>> {
    let output = Command::new("tar")
        .arg("-tzf")
        .arg(archive)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("tar could not list the archive"));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim_start_matches("./").to_string())
        .filter(|line| safe_member(line))
        .collect())
}

fn read_member(archive: &Path, member: &str) -> io::Result<String> {
    if !safe_member(member) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe archive member",
        ));
    }
    let output = Command::new("tar")
        .arg("-xzOf")
        .arg(archive)
        .arg("--")
        .arg(member)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("tar could not read the archive"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A member must stay inside the archive: no absolute path, no parent step.
fn safe_member(member: &str) -> bool {
    !member.is_empty()
        && !member.starts_with('/')
        && !member.starts_with('-')
        && !member.split('/').any(|part| part == "..")
}

fn read_directory(
    directory: &Path,
    lines: &mut Vec<LogLine>,
    warning: &mut bool,
) -> io::Result<()> {
    let mut bodies: BTreeMap<u64, (String, Option<String>)> = BTreeMap::new();
    let Ok(entries) = fs::read_dir(directory) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".log") else {
            continue;
        };
        let body = fs::read_to_string(entry.path())?;
        let times = fs::read_to_string(directory.join(format!("{stem}.times"))).ok();
        push_paired(&mut bodies, stem, body, times);
    }
    for (pane, (body, times)) in bodies {
        push_body(lines, pane, body, times.as_deref(), warning);
    }
    Ok(())
}

fn push_paired(
    bodies: &mut BTreeMap<u64, (String, Option<String>)>,
    stem: &str,
    body: String,
    times: Option<String>,
) {
    let Some(pane) = stem
        .split_once('-')
        .and_then(|(pane, _)| pane.parse::<u64>().ok())
    else {
        return;
    };
    bodies.insert(pane, (body, times));
}

/// Add one file's lines up to the shorter of body and sidecar. A mismatch keeps
/// the shorter run and raises the warning the screen shows.
fn push_body(
    lines: &mut Vec<LogLine>,
    pane: u64,
    body: String,
    times: Option<&str>,
    warning: &mut bool,
) {
    let body: Vec<&str> = body.lines().collect();
    let stamps: Vec<&str> = times.unwrap_or("").lines().collect();
    if times.is_none() || stamps.len() != body.len() {
        *warning = true;
    }
    let count = if times.is_some() {
        body.len().min(stamps.len())
    } else {
        body.len()
    };
    for (index, text) in body.iter().take(count).enumerate() {
        lines.push(LogLine {
            pane,
            time: stamps.get(index).map(|stamp| (*stamp).to_string()),
            text: (*text).to_string(),
        });
    }
}
