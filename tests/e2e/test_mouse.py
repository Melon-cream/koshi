#!/usr/bin/env python3
"""M3 acceptance: click focus, wheel routing, mouse-reporting apps, pagers.

Everything is observed through the real client's PTY. An application that asks
for mouse reports writes what it received to a file, so the test can compare
those bytes with what the server should have encoded.
"""

import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    pane_state,
    ROWS,
    RUNTIME_DIR,
    Client,
    Result,
    clean_runtime,
    cleanup,
    osc52,
    pump_until,
    region,
    send_mouse,
    status_line,
)

TOP_PANE = (1, 11)
# The frame takes the outer row and col; row 11 is the split gutter.
BOTTOM_PANE = (12, ROWS - 2)

# Enables 1000+1006, then records raw input for three seconds without drawing.
APP = r'''
import os, select, sys, time, tty
tty.setraw(0)
log = open(sys.argv[1], "wb", buffering=0)
os.write(1, b"\x1b[?1002h\x1b[?1006hREADY\r\n")
end = time.time() + 3.0
while time.time() < end:
    ready, _, _ = select.select([0], [], [], 0.1)
    if not ready:
        continue
    data = os.read(0, 256)
    if not data:
        break
    log.write(data + b"\n")
os.write(1, b"\x1b[?1002l\x1b[?1006lDONE\r\n")
'''


def begin(name):
    cleanup()
    clean_runtime()
    print("scenario: %s" % name)


def write_file(name, text):
    path = os.path.join(RUNTIME_DIR, name)
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(text)
    return path


def numbered_lines(name, count):
    return write_file(name, "".join("LINE-%03d\n" % i for i in range(1, count + 1)))


def top_number(client):
    line = client.screen.line(0).strip()
    if line.startswith("LINE-"):
        try:
            return int(line[5:])
        except ValueError:
            return -1
    return -1


def wait_top(client, number, timeout=10.0):
    return pump_until(
        client,
        lambda client: top_number(client) == number,
        timeout,
    )


def scenario_click_focus(res):
    begin("click switches the input target")
    client = Client()
    try:
        res.check("status starts in normal mode", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)), status_line(client))
        client.send(b'\x10d')
        res.check("horizontal split shows two panes", pump_until(
            client, lambda c: pane_state(c) == (2, 2)), status_line(client))
        client.send("echo BOT-MARK\n")
        res.check("bottom pane accepts input", pump_until(
            client, lambda c: "BOT-MARK" in region(c, *BOTTOM_PANE)))

        client.send(b"\x10p")
        res.check("leader focus returns to the first pane", pump_until(
            client, lambda c: pane_state(c) == (1, 2)), status_line(client))
        client.send("echo TOP-MARK\n")
        res.check("top pane accepts input", pump_until(
            client, lambda c: "TOP-MARK" in region(c, *TOP_PANE)))

        send_mouse(client, 0, 10, 15)
        res.check("clicking the bottom pane focuses it", pump_until(
            client, lambda c: pane_state(c) == (2, 2)), status_line(client))
        client.send("echo CLICKED-BOTTOM\n")
        res.check("typing after the click reaches the bottom pane", pump_until(
            client, lambda c: "CLICKED-BOTTOM" in region(c, *BOTTOM_PANE)))
        res.check("the top pane did not receive the typed input",
                  "CLICKED-BOTTOM" not in region(client, *TOP_PANE))

        send_mouse(client, 0, 10, 3)
        res.check("clicking the top pane focuses it", pump_until(
            client, lambda c: pane_state(c) == (1, 2)), status_line(client))
        client.send("echo CLICKED-TOP\n")
        res.check("typing after the click reaches the top pane", pump_until(
            client, lambda c: "CLICKED-TOP" in region(c, *TOP_PANE)))
        res.check("the bottom pane did not receive the typed input",
                  "CLICKED-TOP" not in region(client, *BOTTOM_PANE))

        client.send("exit\n")
        res.check("closing the first pane keeps the second", pump_until(
            client, lambda c: pane_state(c) == (1, 1)), status_line(client))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_app_mouse_reporting(res):
    begin("an app with mouse reporting receives click and wheel")
    app_log = os.path.join(RUNTIME_DIR, "mouse.log")
    app = write_file("mouse_app.py", APP)
    client = Client()
    try:
        client.send("for i in $(seq -w 1 40); do echo HIST-$i; done\n")
        res.check("history output is visible", pump_until(
            client, lambda c: "HIST-40" in c.screen.text()))
        client.wait_quiet()
        client.send("python3 %s %s\n" % (app, app_log))
        res.check("the app enables mouse reporting", pump_until(
            client, lambda c: "READY" in c.screen.text()), "READY not seen")
        client.wait_quiet(quiet=0.2)
        before = region(client, 0, ROWS - 1)

        send_mouse(client, 0, 10, 10)
        send_mouse(client, 0, 10, 10, release=True)
        send_mouse(client, 64, 10, 10)
        send_mouse(client, 65, 10, 10)
        time.sleep(0.6)
        client.pump(0.5)

        res.check("the server scrollback did not move on click or wheel",
                  region(client, 0, ROWS - 1) == before)
        res.check("the app is still running", client.exit_code is None)
        res.check("no clipboard write happened", osc52(client) == [])

        res.check("the app exits cleanly", pump_until(
            client, lambda c: "DONE" in c.screen.text(), 8))
        with open(app_log, "rb") as handle:
            reports = handle.read()
        res.check("the app received the left press",
                  b"\x1b[<0;11;11M" in reports, repr(reports))
        res.check("the app received the left release",
                  b"\x1b[<0;11;11m" in reports, repr(reports))
        res.check("the app received the wheel up",
                  b"\x1b[<64;11;11M" in reports, repr(reports))
        res.check("the app received the wheel down",
                  b"\x1b[<65;11;11M" in reports, repr(reports))
        res.check("no report was sent for drags or server-side scrolling",
                  reports.count(b"\x1b[<") == 4, repr(reports))

        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_reporting_in_split(res):
    begin("clicking a reporting pane focuses it and reports pane-local cells")
    app_log = os.path.join(RUNTIME_DIR, "split.log")
    app = write_file("split_app.py", APP)
    client = Client()
    try:
        client.send(b'\x10d')
        pump_until(client, lambda c: pane_state(c) == (2, 2))
        client.send("python3 %s %s\n" % (app, app_log))
        res.check("the app starts in the bottom pane", pump_until(
            client, lambda c: "READY" in region(c, *BOTTOM_PANE)))
        client.send(b"\x10p")
        res.check("leader focus moves to the top pane", pump_until(
            client, lambda c: pane_state(c) == (1, 2)), status_line(client))

        send_mouse(client, 0, 30, 15)
        res.check("clicking the reporting pane focuses it", pump_until(
            client, lambda c: pane_state(c) == (2, 2)), status_line(client))
        send_mouse(client, 0, 30, 15, release=True)
        send_mouse(client, 64, 30, 15)
        res.check("the app finishes", pump_until(
            client, lambda c: "DONE" in region(c, *BOTTOM_PANE), 8))
        with open(app_log, "rb") as handle:
            reports = handle.read()
        res.check("the click is reported in pane-local cells",
                  b"\x1b[<0;30;4M" in reports, repr(reports))
        res.check("the release is reported in pane-local cells",
                  b"\x1b[<0;30;4m" in reports, repr(reports))
        res.check("the wheel is reported in pane-local cells",
                  b"\x1b[<64;30;4M" in reports, repr(reports))

        client.send("exit\n")
        pump_until(client, lambda c: pane_state(c) == (1, 1))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_scrollback(res):
    begin("wheel scrolls history, stdout does not move it, typing returns")
    client = Client()
    try:
        client.send("for i in $(seq -w 1 100); do echo LINE-$i; done\n")
        res.check("100 lines of output", pump_until(
            client, lambda c: "LINE-100" in c.screen.text()))
        client.wait_quiet()
        base = top_number(client)
        res.check("the view starts at the live end", base > 0, "top=%s" % base)

        send_mouse(client, 64, 10, 10)
        res.check("wheel up moves the view by the configured 3 lines",
                  wait_top(client, base - 3), "top=%s" % top_number(client))
        res.check("the newest line leaves the screen",
                  "LINE-100" not in client.screen.text())

        send_mouse(client, 65, 10, 10)
        res.check("wheel down moves back by 3 lines",
                  wait_top(client, base), "top=%s" % top_number(client))

        client.send("(for i in $(seq 101 160); do echo LIVE-$i; sleep 0.02; done) &\n")
        res.check("streaming output starts", pump_until(
            client, lambda c: "LIVE-110" in c.screen.text()))
        send_mouse(client, 64, 10, 10)
        send_mouse(client, 64, 10, 10)
        time.sleep(0.2)
        client.pump(0.2)
        held = top_number(client)
        res.check("the history view is scrolled back", held > 0, "top=%s" % held)
        # The stream keeps running while the view is held in the history.
        time.sleep(1.5)
        client.pump(0.4)
        res.check("streaming output does not move the history view",
                  top_number(client) == held,
                  "before=%s after=%s" % (held, top_number(client)))
        res.check("the attached view does not follow the stream",
                  "LIVE-160" not in client.screen.text())

        client.send("\r")
        res.check("a key returns the view to the live end", pump_until(
            client, lambda c: "LIVE-160" in c.screen.text(), 10))

        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_configured_scroll_lines(res):
    begin("wheel step follows scroll_lines from the configuration file")
    config_home = os.path.join(RUNTIME_DIR, "config-home")
    os.makedirs(os.path.join(config_home, "koshi"), exist_ok=True)
    with open(os.path.join(config_home, "koshi", "config.toml"), "w", encoding="utf-8") as handle:
        handle.write("scroll_lines = 5\n")
    lines = numbered_lines("pager5.txt", 200)
    client = Client(env={"XDG_CONFIG_HOME": config_home})
    try:
        client.send("for i in $(seq -w 1 100); do echo LINE-$i; done\n")
        res.check("history output is visible", pump_until(
            client, lambda c: "LINE-100" in c.screen.text()))
        client.wait_quiet()
        base = top_number(client)
        send_mouse(client, 64, 10, 10)
        res.check("the configured 5 lines are scrolled per notch",
                  wait_top(client, base - 5), "top=%s" % top_number(client))
        send_mouse(client, 65, 10, 10)
        res.check("wheel down uses the same step", wait_top(client, base),
                  "top=%s" % top_number(client))

        client.send("less -R %s\n" % lines)
        res.check("the pager starts at the first line", pump_until(
            client, lambda c: top_number(c) == 1))
        client.wait_quiet(quiet=0.2)
        send_mouse(client, 65, 10, 10)
        res.check("the pager uses the configured step too",
                  wait_top(client, 6), "top=%s" % top_number(client))
        send_mouse(client, 64, 10, 10)
        res.check("the pager uses the configured step upward",
                  wait_top(client, 1), "top=%s" % top_number(client))
        client.send("q")
        pump_until(client, lambda c: top_number(c) < 0)
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_pager_wheel(res):
    begin("wheel in an alternate-screen pager sends arrow keys")
    lines = numbered_lines("pager.txt", 200)
    client = Client()
    try:
        client.send("less -R %s\n" % lines)
        res.check("less opens in the alternate screen", pump_until(
            client, lambda c: top_number(c) == 1))
        client.wait_quiet(quiet=0.2)

        send_mouse(client, 65, 10, 10)
        res.check("wheel down advances the pager by 3 lines",
                  wait_top(client, 4), "top=%s" % top_number(client))
        send_mouse(client, 65, 10, 10)
        res.check("a second notch advances 3 more lines",
                  wait_top(client, 7), "top=%s" % top_number(client))
        send_mouse(client, 64, 10, 10)
        res.check("wheel up goes back 3 lines",
                  wait_top(client, 4), "top=%s" % top_number(client))

        client.send("q")
        res.check("less quits back to the shell", pump_until(
            client, lambda c: top_number(c) < 0), "top=%r" % client.screen.line(0))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_terminal_restore(res):
    begin("mouse tracking is enabled on attach and cleared on detach")
    client = Client()
    second = None
    try:
        res.check("the client enables SGR button-event tracking",
                  pump_until(client, lambda c: b"\x1b[?1002h\x1b[?1006h" in c.raw))
        pump_until(client, lambda c: "[NORMAL]" in status_line(c))
        client.send(b"\x10")
        res.check("the pane leader group is pending", pump_until(
            client, lambda c: "[PANE]" in status_line(c)), status_line(client))
        send_mouse(client, 0, 10, 10)
        res.check("a click clears the pending leader", pump_until(
            client, lambda c: "[NORMAL]" in status_line(c)), status_line(client))
        client.send("echo LEADER-CLEARED\n")
        res.check("typing after the click still reaches the shell",
                  pump_until(client, lambda c: "LEADER-CLEARED" in c.screen.text()))
        client.detach_keys()
        res.check("detach exits cleanly", client.wait_exit(10) == 0)
        res.check("the client disables mouse tracking before exiting",
                  b"\x1b[?1006l\x1b[?1002l" in client.raw)

        second = Client()
        res.check("reattach enables mouse tracking again", pump_until(
            second, lambda c: b"\x1b[?1002h\x1b[?1006h" in c.raw))
        res.check("the reattached client shows normal mode", pump_until(
            second, lambda c: "[NORMAL] main" in status_line(c)), status_line(second))
        second.detach_keys()
        res.check("the second detach exits cleanly", second.wait_exit(10) == 0)
    finally:
        client.close()
        if second is not None:
            second.close()


def main():
    res = Result()
    try:
        scenario_click_focus(res)
        scenario_app_mouse_reporting(res)
        scenario_reporting_in_split(res)
        scenario_scrollback(res)
        scenario_configured_scroll_lines(res)
        scenario_pager_wheel(res)
        scenario_terminal_restore(res)
    finally:
        cleanup()
    return res.report("click, wheel routing, mouse reporting, pager")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
