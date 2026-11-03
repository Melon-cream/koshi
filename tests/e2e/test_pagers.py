#!/usr/bin/env python3
"""M4 acceptance: FR-6 in the pagers people actually use.

less, man, git log, and a mouse-reporting vim. The wheel has to reach the
application while it owns the alternate screen, and a click has to be handled by
the application rather than by the multiplexer.
"""

import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    RUNTIME_DIR,
    Client,
    Result,
    clean_runtime,
    cleanup,
    pump_until,
    send_mouse,
    status_line,
)

WHEEL_UP = 64
WHEEL_DOWN = 65


def begin(name):
    cleanup()
    clean_runtime()
    print("scenario: %s" % name)


def notch(client, code, count=1, col=10, row=10):
    for _ in range(count):
        send_mouse(client, code, col, row)
        client.pump(0.02)


def wheel_until(client, code, predicate, batches=25, per_batch=4, timeout=2.0):
    """Wheel in small batches until `predicate` holds.

    Returns the number of notches used, or -1 when the cap was hit. Wheel events
    are not replayed by the test: only the wheel gets the view there.
    """
    for index in range(1, batches + 1):
        notch(client, code, per_batch)
        if pump_until(client, predicate, timeout):
            return index * per_batch
    return -1


def top_line(client):
    return client.screen.line(0).rstrip()


def bottom_line(client):
    return client.screen.line(client.screen.rows - 2).rstrip()


def wait_top(client, needle, timeout=10.0):
    return pump_until(client, lambda c: needle in top_line(c), timeout)


def scenario_less(res):
    begin("less: the wheel alone reads the file to the end")
    path = os.path.join(RUNTIME_DIR, "big.txt")
    with open(path, "w", encoding="utf-8") as handle:
        handle.write("".join("LESS-%03d\n" % index for index in range(1, 301)))
    client = Client()
    try:
        res.check("the shell is ready", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)))
        client.send("less -R %s\n" % path)
        res.check("less shows the first line", wait_top(client, "LESS-001"), top_line(client))
        client.wait_quiet(quiet=0.2)

        notch(client, WHEEL_DOWN, 90)
        res.check("90 notches scroll 270 lines", wait_top(client, "LESS-271"), top_line(client))
        used = wheel_until(client, WHEEL_DOWN,
                           lambda c: "LESS-300" in c.screen.text() and "(END)" in bottom_line(c))
        res.check("the end of the file is reachable with the wheel", used > 0,
                  "notches=%d bottom=%r" % (used, bottom_line(client)))
        used = wheel_until(client, WHEEL_UP, lambda c: "LESS-300" not in c.screen.text(),
                           batches=10)
        res.check("the wheel also goes back", used > 0,
                  "notches=%d bottom=%r" % (used, bottom_line(client)))

        client.send("q")
        res.check("less quits back to the shell", pump_until(
            client, lambda c: "[NORMAL]" in status_line(c) and "LESS-" not in top_line(c)))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_man(res):
    begin("man: the wheel moves the page")
    client = Client()
    try:
        res.check("the shell is ready", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)))
        client.send("man ls\n")
        res.check("man renders the page", wait_top(client, "LS(1)"), top_line(client))
        client.wait_quiet(quiet=0.3)

        notch(client, WHEEL_DOWN, 2)
        res.check("the wheel scrolls the manual page", pump_until(
            client, lambda c: "LS(1)" not in top_line(c)), top_line(client))
        notch(client, WHEEL_UP, 2)
        res.check("the wheel scrolls the manual page back", wait_top(client, "LS(1)"),
                  top_line(client))

        client.send("q")
        res.check("man quits back to the shell", pump_until(
            client, lambda c: "LS(1)" not in c.screen.text() and "[NORMAL]" in status_line(c)),
            top_line(client))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def make_repo(count=40):
    repo = os.path.join(RUNTIME_DIR, "repo")
    os.makedirs(repo, exist_ok=True)
    base = ["git", "-c", "user.email=e2e@example.com", "-c", "user.name=e2e", "-C", repo]
    subprocess.run(["git", "init", "-q", repo], check=True)
    for index in range(1, count + 1):
        subprocess.run(base + ["commit", "-q", "--allow-empty", "-m", "COMMIT-%03d" % index],
                       check=True)
    return repo


def commit_at_top(client):
    match = re.search(r"COMMIT-(\d{3})", top_line(client))
    return int(match.group(1)) if match else -1


def scenario_git_log(res):
    begin("git log: the wheel reads the history")
    try:
        repo = make_repo()
    except (OSError, subprocess.CalledProcessError) as err:
        print("  SKIP git is not usable in this container (%s)" % err)
        return
    client = Client()
    try:
        res.check("the shell is ready", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)))

        print("  part 1: a pager on the alternate screen")
        # git exports LESS=FRX, and -X keeps less off the alternate screen.
        client.send("cd %s && LESS= git -c core.pager='less -R' log --oneline\n" % repo)
        res.check("git log starts at the newest commit", pump_until(
            client, lambda c: commit_at_top(c) == 40, 15), top_line(client))
        client.wait_quiet(quiet=0.3)
        notch(client, WHEEL_DOWN, 2)
        res.check("the wheel moves down the log", pump_until(
            client, lambda c: 0 < commit_at_top(c) < 40), top_line(client))
        used = wheel_until(client, WHEEL_DOWN, lambda c: "COMMIT-001" in c.screen.text())
        res.check("the oldest commit is reachable with the wheel", used > 0,
                  "notches=%d top=%r" % (used, top_line(client)))
        client.send("q")
        res.check("the pager quits back to the shell", pump_until(
            client, lambda c: "COMMIT-" not in c.screen.text()))

        print("  part 2: git's own pager (less -X) stays in the normal screen")
        client.send("git log --oneline\n")
        res.check("git log draws the log", pump_until(
            client, lambda c: "COMMIT-040" in c.screen.text(), 15))
        client.wait_quiet(quiet=0.3)
        before = top_line(client)
        used = wheel_until(client, WHEEL_UP, lambda c: top_line(c) != before, batches=5)
        res.check("the wheel scrolls the pane history for a -X pager", used > 0,
                  "notches=%d top=%r" % (used, top_line(client)))
        client.send("q")
        client.send("\r")
        res.check("the pager still quits back to the shell", pump_until(
            client, lambda c: "[NORMAL]" in status_line(c)), status_line(client))
        client.send("echo BACK-TO-SHELL\n")
        res.check("the pane accepts input after the pager", pump_until(
            client, lambda c: "BACK-TO-SHELL" in c.screen.text()))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def vim_line_at(client, row):
    match = re.match(r"\s*(\d+)", client.screen.line(row))
    return int(match.group(1)) if match else -1


def scenario_vim_mouse(res):
    begin("vim with mouse=a handles clicks and the wheel itself")
    path = os.path.join(RUNTIME_DIR, "vim.txt")
    with open(path, "w", encoding="utf-8") as handle:
        handle.write("".join("VIM-LINE-%03d\n" % index for index in range(1, 201)))
    client = Client()
    try:
        res.check("the shell is ready", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)))
        client.send("vim -u NONE -c 'set mouse=a' -c 'set number' %s\n" % path)
        res.check("vim starts on the first line", pump_until(
            client, lambda c: vim_line_at(c, 0) == 1), client.screen.line(0))
        client.wait_quiet(quiet=0.3)

        send_mouse(client, 0, 20, 14)
        send_mouse(client, 0, 20, 14, release=True)
        client.pump(0.3)
        client.send(":echo line('.')")
        client.send(b"\r")
        res.check("a click moved vim's cursor to line 15", pump_until(
            client, lambda c: c.screen.line(22).strip().endswith("15"), 10),
            "row=%r" % client.screen.line(22))

        client.send(b"\r")
        client.pump(0.2)
        notch(client, WHEEL_DOWN, 1, col=20, row=5)
        res.check("the wheel scrolled vim's view, not the server history", pump_until(
            client, lambda c: vim_line_at(c, 0) > 1, 10), "top=%r" % client.screen.line(0))
        notch(client, WHEEL_UP, 1, col=20, row=5)
        res.check("the wheel scrolls vim back", pump_until(
            client, lambda c: vim_line_at(c, 0) == 1, 10), "top=%r" % client.screen.line(0))

        client.send(b"\x1b")
        client.send(":q!\r")
        res.check("vim quits back to the shell", pump_until(
            client, lambda c: "VIM-LINE-001" not in c.screen.text(), 10),
            "top=%r" % client.screen.line(0))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def main():
    res = Result()
    try:
        scenario_less(res)
        scenario_man(res)
        scenario_git_log(res)
        scenario_vim_mouse(res)
    finally:
        cleanup()
    return res.report("pagers: less, man, git log, vim mouse")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
