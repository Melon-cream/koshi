#!/usr/bin/env python3
"""FR-9 acceptance: detach, reattach, concurrent clients, and process survival.

The screen checks compare the whole visible screen, not its length. Trailing
padding and trailing blank lines are dropped; everything else, including the
column position of every character and every internal blank line, must match.
Cursor position is not part of the text at all.

Scenario 1: one client runs commands, detaches, reattaches, and must see exactly
the same screen again. The shell then exits and the server must go away.

Scenario 2: two clients attach to the same server at the same time. They must
show the same state, input from either one must reach the one pane (and thus
both screens), and detaching one must leave the other connected. After both
detach the server and pane process must still be alive.
"""

import os
import signal
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    Client,
    Result,
    clean_runtime,
    cleanup,
    pump_until,
    server_alive,
)

MARKER = "M1-MARKER-%d" % os.getpid()


def pid_alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def pid_from(screen, prefix):
    for line in screen.splitlines():
        if prefix in line:
            tail = line.split(prefix)[-1].strip()
            digits = "".join(ch for ch in tail if ch.isdigit())
            if digits:
                return int(digits)
    return 0


def first_difference(before, after):
    left = before.split("\n")
    right = after.split("\n")
    for index in range(max(len(left), len(right))):
        a = left[index] if index < len(left) else "<missing line>"
        b = right[index] if index < len(right) else "<missing line>"
        if a != b:
            return "line %d: before=%r after=%r" % (index + 1, a, b)
    return "no difference"


def wait_gone(timeout=10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if not server_alive():
            return True
        time.sleep(0.1)
    return False


def scenario_single_client(res):
    clean_runtime()
    print("scenario 1: one client, exact screen reconstruction")

    first = Client()
    second = None
    try:
        first.send("echo %s\n" % MARKER)
        # The typed line is echoed by the tty, so a marker that only appears once
        # means no shell ran the command.
        res.check("the shell runs the typed command", pump_until(
            first, lambda c: c.screen.text().count(MARKER) >= 2, 10),
            "screen=%r" % first.screen.text()[:300])

        first.send("sleep 300 & echo SLEEPER=$!\n")
        screen = first.wait_for("SLEEPER=", 10)
        res.check("background process started", screen is not None)
        sleeper = pid_from(screen or "", "SLEEPER=")
        res.check("sleeper pid captured", sleeper > 0, "pid=%s" % sleeper)

        before = first.stable_screen()
        res.check("pre-detach screen is not empty", any(line.strip() for line in before.splitlines()),
                  "screen=%r" % before[:400])
        res.check("pre-detach screen keeps the marker", MARKER in before)

        first.detach_keys()
        code = first.wait_exit(10)
        res.check("detach exits the client", code == 0, "exit=%r" % code)
        res.check("server process survives detach", server_alive())
        res.check("pane process survives detach", pid_alive(sleeper), "pid=%s" % sleeper)

        print("scenario 1: reattach")
        second = Client()
        res.check("reattached screen keeps prior content", second.wait_for(MARKER, 10) is not None)
        after = second.stable_screen()
        res.check("reattached screen equals the detached screen", after == before,
                  first_difference(before, after))

        second.send("echo REATTACH-OK\n")
        res.check("input works after reattach", second.wait_for("REATTACH-OK", 10) is not None)

        second.send("exit\n")
        code = second.wait_exit(10)
        res.check("client exits when shell exits", code is not None, "exit=%r" % code)
    finally:
        first.close()
        if second is not None:
            second.close()

    res.check("server exits after the last pane closes", wait_gone())


def scenario_concurrent_clients(res):
    clean_runtime()
    print("scenario 2: two concurrent clients")

    alice = Client()
    bob = None
    carol = None
    try:
        alice.send("echo CONCURRENT-BASE\n")
        res.check("first client shows shared output", alice.wait_for("CONCURRENT-BASE", 10) is not None)

        bob = Client()
        res.check("second client sees the same pane", bob.wait_for("CONCURRENT-BASE", 10) is not None)

        a_screen = alice.stable_screen()
        b_screen = bob.stable_screen()
        res.check("both clients show the same screen", a_screen == b_screen,
                  first_difference(a_screen, b_screen))

        bob.send("echo FROM-BOB\n")
        seen_bob = bob.wait_for("FROM-BOB", 10) is not None
        seen_on_alice = alice.wait_for("FROM-BOB", 10) is not None
        res.check("bob's input reaches bob", seen_bob)
        res.check("bob's input reaches alice", seen_on_alice)
        a_screen = alice.stable_screen()
        b_screen = bob.stable_screen()
        res.check("screens still match after bob typed", a_screen == b_screen,
                  first_difference(a_screen, b_screen))

        alice.send("echo FROM-ALICE\n")
        seen_alice = alice.wait_for("FROM-ALICE", 10) is not None
        seen_on_bob = bob.wait_for("FROM-ALICE", 10) is not None
        res.check("alice's input reaches alice", seen_alice)
        res.check("alice's input reaches bob", seen_on_bob)

        alice.send("sleep 300 & echo CONCURRENT-SLEEPER=$!\n")
        screen = alice.wait_for("CONCURRENT-SLEEPER=", 10)
        res.check("concurrent sleeper started", screen is not None)
        sleeper = pid_from(screen or "", "CONCURRENT-SLEEPER=")
        res.check("concurrent sleeper pid captured", sleeper > 0, "pid=%s" % sleeper)

        bob.wait_for("CONCURRENT-SLEEPER=", 10)
        b_before_detach = bob.stable_screen()

        alice.detach_keys()
        code = alice.wait_exit(10)
        res.check("first concurrent client detaches", code == 0, "exit=%r" % code)
        res.check("server survives first detach", server_alive())
        res.check("pane process survives first detach", pid_alive(sleeper), "pid=%s" % sleeper)

        res.check("second client stays connected", bob.exit_code is None)
        bob.send("echo BOB-STILL-CONNECTED\n")
        res.check("second client is still usable", bob.wait_for("BOB-STILL-CONNECTED", 10) is not None)
        res.check("second client did not exit", bob.exit_code is None)

        b_final = bob.stable_screen()
        bob.detach_keys()
        code = bob.wait_exit(10)
        res.check("second concurrent client detaches", code == 0, "exit=%r" % code)
        res.check("server survives the last detach", server_alive())
        res.check("pane process survives the last detach", pid_alive(sleeper), "pid=%s" % sleeper)

        print("scenario 2: reattach after both clients detached")
        carol = Client()
        res.check("third client sees the preserved pane", carol.wait_for("CONCURRENT-SLEEPER", 10) is not None)
        c_screen = carol.stable_screen()
        res.check("reattached concurrent screen equals the last one", c_screen == b_final,
                  first_difference(b_final, c_screen))

        carol.send("echo REATTACH-CONCURRENT\n")
        res.check("third client accepts input", carol.wait_for("REATTACH-CONCURRENT", 10) is not None)

        carol.send("exit\n")
        code = carol.wait_exit(10)
        res.check("third client exits when shell exits", code is not None, "exit=%r" % code)
    finally:
        alice.close()
        if bob is not None:
            bob.close()
        if carol is not None:
            carol.close()

    res.check("server exits after the concurrent scenario", wait_gone())


def main():
    res = Result()
    try:
        scenario_single_client(res)
        scenario_concurrent_clients(res)
    finally:
        cleanup()
    return res.report("detach and attach")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
