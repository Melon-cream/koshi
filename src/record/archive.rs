//! Weekly packing of finished recordings into one archive per week.
//!
//! The step runs in the background while the server keeps writing. It packs
//! with the system `tar` in one call, then removes the source files that did not
//! change while the archive was being written. A week that already has its
//! archive is skipped, so the step is idempotent.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use super::{Date, local_offset_seconds, parse_workspace_dir};

/// How many finished weeks one run looks back at.
const WEEKS: i64 = 8;

pub(crate) struct Outcome {
    pub(crate) archived: Vec<PathBuf>,
    pub(crate) sources_removed: usize,
    pub(crate) expired: usize,
}

pub(crate) fn run(root: &Path, retain_days: u32) -> io::Result<Outcome> {
    let today = Date::today(local_offset_seconds());
    let this_monday = today.monday().days();
    let mut outcome = Outcome {
        archived: Vec::new(),
        sources_removed: 0,
        expired: 0,
    };
    for week in 1..=WEEKS {
        let start = Date::from_days(this_monday - 7 * week);
        let end = Date::from_days(start.days() + 6);
        let archive =
            root.join(start.month())
                .join(format!("{}-{}.tar.gz", start.compact(), end.compact()));
        if archive.exists() {
            continue;
        }
        match archive_week(root, start, &archive) {
            Ok(removed) => {
                if removed.is_some() {
                    outcome.archived.push(archive);
                    outcome.sources_removed += removed.unwrap_or(0);
                }
            }
            Err(err) => {
                eprintln!(
                    "koshi: cannot archive the recordings of {}: {err}",
                    start.compact()
                );
            }
        }
    }
    if retain_days > 0 {
        outcome.expired = expire(root, today, retain_days)?;
    }
    Ok(outcome)
}

/// Pack one finished week. `Ok(None)` means the week had no recording.
fn archive_week(root: &Path, start: Date, archive: &Path) -> io::Result<Option<usize>> {
    let members = collect(root, start)?;
    if members.is_empty() {
        return Ok(None);
    }
    let directory = archive.parent().unwrap_or(root);
    fs::create_dir_all(directory)?;
    let tmp = tmp_path(archive);
    let tar_start = SystemTime::now();
    let mut command = Command::new("tar");
    command
        .arg("-czf")
        .arg(&tmp)
        .arg("-C")
        .arg(root)
        .arg("--")
        .args(&members)
        .stdin(Stdio::null());
    let status = command.status();
    let failed = match status {
        Ok(status) => !status.success(),
        Err(_) => true,
    };
    if failed {
        let _ = fs::remove_file(&tmp);
        return Err(io::Error::other("tar failed"));
    }
    fs::rename(&tmp, archive)?;
    Ok(Some(remove_sources(root, start, &members, tar_start)))
}

/// Relative paths of the week's bodies and sidecars, sorted for a stable order.
fn collect(root: &Path, start: Date) -> io::Result<Vec<PathBuf>> {
    let mut members = Vec::new();
    for day in 0..7 {
        let date = Date::from_days(start.days() + day);
        let day_dir = root.join(date.compact());
        let entries = match fs::read_dir(&day_dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        for entry in entries.flatten() {
            if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                continue;
            }
            let workspace = entry.file_name();
            let workspace = workspace.to_string_lossy().into_owned();
            if parse_workspace_dir(&workspace).is_none() {
                continue;
            }
            let files = match fs::read_dir(entry.path()) {
                Ok(files) => files,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            };
            for file in files.flatten() {
                if !file.file_type().map(|kind| kind.is_file()).unwrap_or(false) {
                    continue;
                }
                let name = file.file_name();
                let name = name.to_string_lossy();
                if !(name.ends_with(".log") || name.ends_with(".times")) {
                    continue;
                }
                members.push(
                    PathBuf::from(date.compact())
                        .join(&workspace)
                        .join(name.as_ref()),
                );
            }
        }
    }
    members.sort();
    Ok(members)
}

fn remove_sources(root: &Path, start: Date, members: &[PathBuf], tar_start: SystemTime) -> usize {
    let mut removed = 0;
    for member in members {
        let path = root.join(member);
        let modified = match fs::metadata(&path).and_then(|meta| meta.modified()) {
            Ok(modified) => modified,
            Err(_) => continue,
        };
        if modified > tar_start {
            continue;
        }
        if fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    for day in 0..7 {
        let date = Date::from_days(start.days() + day);
        let day_dir = root.join(date.compact());
        let entries = match fs::read_dir(&day_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                let _ = fs::remove_dir(entry.path());
            }
        }
        let _ = fs::remove_dir(&day_dir);
    }
    removed
}

/// Remove archives older than `retain_days`, counted from the week start.
fn expire(root: &Path, today: Date, retain_days: u32) -> io::Result<usize> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    let mut expired = 0;
    for entry in entries.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let directory = entry.path();
        let files = match fs::read_dir(&directory) {
            Ok(files) => files,
            Err(_) => continue,
        };
        for file in files.flatten() {
            let name = file.file_name();
            let name = name.to_string_lossy();
            let Some(stem) = name.strip_suffix(".tar.gz") else {
                continue;
            };
            let Some(start) = stem
                .split_once('-')
                .and_then(|(start, _)| Date::parse(start))
            else {
                continue;
            };
            if today.days() - start.days() > retain_days as i64
                && fs::remove_file(file.path()).is_ok()
            {
                expired += 1;
            }
        }
        let _ = fs::remove_dir(&directory);
    }
    Ok(expired)
}

fn tmp_path(archive: &Path) -> PathBuf {
    let mut name = archive.as_os_str().to_os_string();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_week_is_labelled_by_the_month_of_its_start() {
        let start = Date {
            year: 2026,
            month: 8,
            day: 31,
        };
        assert_eq!(start.month(), "202608");
        assert_eq!(start.compact(), "20260831");
    }

    #[test]
    fn a_partial_name_is_not_an_archive_of_a_week() {
        assert_eq!(
            Date::parse("20260810"),
            Some(Date {
                year: 2026,
                month: 8,
                day: 10,
            })
        );
        assert_eq!(Date::parse("nonsense"), None);
    }
}
