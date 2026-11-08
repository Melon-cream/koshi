#!/usr/bin/env python3
"""Weekly archiving of finished recordings: the archive, its members, retention.

A finished calendar week is packed into `<root>/<YYYYMM>/<start>-<end>.tar.gz`
and its source files are deleted. The suite writes the source files by hand, so
the week it exercises is deterministic, then starts the real server and reads
the archive with `tar` the way the design says it is written.

Retention counts from the week start: an archive within `record_retain_days` is
kept, an older one is removed in the same pass.
"""

import datetime
import glob
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    RUNTIME_DIR,
    Client,
    Result,
    clean_runtime,
    cleanup,
    pump_until,
    status_line,
)

RECORD_ROOT = os.path.join(RUNTIME_DIR, "record")
CONFIG_DIR = os.path.join(RUNTIME_DIR, "config")


def begin(name):
    cleanup()
    clean_runtime()
    print("scenario: %s" % name)


def wait_status(client, needle, timeout=10.0):
    return pump_until(client, lambda c: needle in status_line(c), timeout)


def wait_for(predicate, timeout=20.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if predicate():
            return True
        time.sleep(0.05)
    return predicate()


def last_week(today=None):
    """Monday and Sunday of the calendar week before today."""
    today = today or datetime.date.today()
    monday = today - datetime.timedelta(days=today.weekday())
    start = monday - datetime.timedelta(days=7)
    return start, start + datetime.timedelta(days=6)


def pack_label(day):
    return day.strftime("%Y%m%d")


def archive_path(start):
    return os.path.join(RECORD_ROOT, start.strftime("%Y%m"), "%s-%s.tar.gz"
                        % (pack_label(start), pack_label(start + datetime.timedelta(days=6))))


def write_recording(day, workspace, pane, lines, hour=12):
    directory = os.path.join(RECORD_ROOT, pack_label(day), workspace)
    os.makedirs(directory, mode=0o700, exist_ok=True)
    body = os.path.join(directory, "%d-%02d0000.log" % (pane, hour))
    times = body[: -len(".log")] + ".times"
    with open(body, "w", encoding="utf-8") as handle:
        handle.write("\n".join(lines) + "\n")
    with open(times, "w", encoding="utf-8") as handle:
        for index in range(len(lines)):
            handle.write("%sT%02d:00:%02d+09:00\n" % (day.isoformat(), hour, index))
    return body, times


def make_archive(start, text):
    """A valid archive for the week starting `start`, with one member."""
    directory = os.path.join(RECORD_ROOT, start.strftime("%Y%m"))
    os.makedirs(directory, mode=0o700, exist_ok=True)
    staging = os.path.join(RUNTIME_DIR, "staging-%s" % pack_label(start))
    member = os.path.join(staging, pack_label(start), "main-1")
    os.makedirs(member, exist_ok=True)
    with open(os.path.join(member, "1-120000.log"), "w", encoding="utf-8") as handle:
        handle.write(text + "\n")
    path = archive_path(start)
    subprocess.run(["tar", "-czf", path, "-C", staging, "."], check=True)
    return path


def members(path):
    out = subprocess.run(["tar", "-tzf", path], check=True, capture_output=True, text=True)
    names = []
    for line in out.stdout.splitlines():
        name = line.lstrip("./")
        if name:
            names.append(name)
    return names


def member_text(path, member):
    out = subprocess.run(["tar", "-xzOf", path, member], check=True, capture_output=True,
                         text=True)
    return out.stdout


def kill_server():
    from harness import server_pids
    import signal

    for pid in server_pids():
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass
    wait_for(lambda: not server_pids(), timeout=5.0)


def scenario_archives_last_week(res):
    begin("the finished week becomes one archive and its files are removed")
    start, end = last_week()
    day = start + datetime.timedelta(days=2)
    body, times = write_recording(day, "main-1", 1, ["ARCHIVED-ONE", "ARCHIVED-TWO"])
    body_text = open(body, encoding="utf-8").read()
    times_text = open(times, encoding="utf-8").read()
    archive = archive_path(start)

    client = Client()
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        res.check("the finished week is archived", wait_for(lambda: os.path.exists(archive)),
                  archive)
        res.check("no partial archive is left behind",
                  not glob.glob(os.path.join(RECORD_ROOT, "*", "*.tmp")))
        if os.path.exists(archive):
            listed = members(archive)
            wanted = "%s/main-1/1-120000" % pack_label(day)
            res.check("the archive keeps the day and workspace path",
                      wanted + ".log" in listed and wanted + ".times" in listed,
                      repr(listed))
            res.check("the body round-trips through the archive",
                      member_text(archive, wanted + ".log") == body_text,
                      repr(member_text(archive, wanted + ".log")))
            res.check("the sidecar round-trips through the archive",
                      member_text(archive, wanted + ".times") == times_text,
                      repr(member_text(archive, wanted + ".times")))
            res.check("the archive lives under the month of the week start",
                      os.path.dirname(archive) == os.path.join(RECORD_ROOT, start.strftime("%Y%m")),
                      os.path.dirname(archive))
        res.check("the source files are removed", not os.path.exists(body) and not os.path.exists(times),
                  body)
        res.check("the emptied day directory is removed",
                  not os.path.exists(os.path.join(RECORD_ROOT, pack_label(day))),
                  os.path.join(RECORD_ROOT, pack_label(day)))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_retention(res):
    begin("record_retain_days drops archives older than the week start allows")
    os.makedirs(os.path.join(CONFIG_DIR, "koshi"), mode=0o700, exist_ok=True)
    with open(os.path.join(CONFIG_DIR, "koshi", "config.toml"), "w", encoding="utf-8") as handle:
        handle.write("record_retain_days = 7\n")

    today = datetime.date.today()
    this_monday = today - datetime.timedelta(days=today.weekday())
    old_start = this_monday - datetime.timedelta(days=200)
    old_start -= datetime.timedelta(days=old_start.weekday())
    fresh_start = this_monday
    old = make_archive(old_start, "OLD-ARCHIVE")
    fresh = make_archive(fresh_start, "FRESH-ARCHIVE")

    client = Client(env={"XDG_CONFIG_HOME": CONFIG_DIR})
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        res.check("an expired archive is deleted", wait_for(lambda: not os.path.exists(old)), old)
        res.check("an archive inside the window survives", os.path.exists(fresh), fresh)
        res.check("the surviving archive is readable",
                  os.path.exists(fresh) and "FRESH-ARCHIVE" in member_text(fresh, members(fresh)[0]),
                  fresh)
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_show_reads_archive(res):
    begin("koshi record show reads a finished week from its archive")
    start, _ = last_week()
    day = start + datetime.timedelta(days=3)
    write_recording(day, "main-1", 7, ["ARCHIVED-SHOW-LINE"])
    archive = archive_path(start)

    client = Client()
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status_line(client))
        res.check("the finished week is archived", wait_for(lambda: os.path.exists(archive)),
                  archive)
    finally:
        client.close()
    kill_server()

    viewer = Client(args=("record", "show", "main", day.isoformat()))
    try:
        res.check("the archived day opens directly",
                  pump_until(viewer, lambda c: "ARCHIVED-SHOW-LINE" in c.screen.text(), 10.0),
                  viewer.screen.text())
        res.check("the archived line keeps its pane and time",
                  pump_until(viewer, lambda c: "p7" in c.screen.text(), 5.0),
                  viewer.screen.text())
        viewer.send("q")
        res.check("the viewer quits", viewer.wait_exit(10) is not None, viewer.exit_code)
    finally:
        viewer.close()


def main():
    res = Result()
    try:
        scenario_archives_last_week(res)
        scenario_retention(res)
        scenario_show_reads_archive(res)
    finally:
        cleanup()
    return res.report("weekly recording archive")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
