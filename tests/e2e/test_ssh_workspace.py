#!/usr/bin/env python3
"""SSH workspace acceptance: alias, settings screen, ssh launch, and restore.

The suite never reaches a network. `ssh` on PATH is a script that records its
own argv to a log file, so "one OS argument, no shell" is checked against what
the pane actually executed rather than against a message the implementation
chose to print.

Every scenario starts from an empty runtime directory. The format-1 fixture in
`fixtures/state-v1.bin` was written by the 0.1.1 binary (two workspaces, the
first tab renamed `logs` and split, the second tab renamed `build`) and is read
only; a scenario copies it into the runtime directory.
"""

import os
import shutil
import signal
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    ROWS,
    RUNTIME_DIR,
    Client,
    Result,
    clean_runtime,
    cleanup,
    normalize_screen,
    pump_until,
    server_pids,
)

PANE = b"\x10"
SESSION = b"\x0f"
ARROW_UP = b"\x1b[A"
ARROW_DOWN = b"\x1b[B"
ENTER = b"\r"
ESC = b"\x1b"
BACKSPACE = b"\x7f"

STATE = os.path.join(RUNTIME_DIR, "state.bin")
FIXTURE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures", "state-v1.bin")
MAGIC = b"KOSHI\x00"
FORMAT_V2 = b"\x02"

FAKE_SSH = """#!/bin/sh
log=${KOSHI_SSH_LOG:?}
if [ "$1" = "-G" ]; then
  printf 'G' >> "$log"
  for arg in "$@"; do printf '\\t<%s>' "$arg" >> "$log"; done
  printf '\\n' >> "$log"
  if [ "$2" = "missing-alias" ]; then
    printf 'fake ssh: no such host\\n' >&2
    exit 255
  fi
  printf 'hostname %s\\n' "$2"
  exit 0
fi
printf 'RUN' >> "$log"
for arg in "$@"; do printf '\\t<%s>' "$arg" >> "$log"; done
printf '\\n' >> "$log"
printf 'SSH-PANE:%s\\n' "$1"
exec sleep 300
"""


def begin(name):
    cleanup()
    clean_runtime()
    print("scenario: %s" % name)


def status(client):
    return client.screen.line(ROWS - 1)


def wait_status(client, needle, timeout=10.0):
    return pump_until(client, lambda c: needle in status(c), timeout)


def wait_text(client, needle, timeout=10.0):
    return pump_until(client, lambda c: needle in c.screen.text(), timeout)


def fake_ssh_env():
    """PATH with the recording `ssh` first, and where it writes its argv log."""
    bindir = os.path.join(RUNTIME_DIR, "bin")
    os.makedirs(bindir, exist_ok=True)
    path = os.path.join(bindir, "ssh")
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(FAKE_SSH)
    os.chmod(path, 0o755)
    log = os.path.join(RUNTIME_DIR, "ssh.log")
    open(log, "w").close()
    return {
        "PATH": bindir + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"),
        "KOSHI_SSH_LOG": log,
    }, log


def ssh_calls(log):
    """Lines the fake ssh recorded, one per invocation."""
    try:
        with open(log, "r", encoding="utf-8") as handle:
            return [line.rstrip("\n") for line in handle if line.strip()]
    except OSError:
        return []


def ssh_runs(log):
    return [line for line in ssh_calls(log) if line.startswith("RUN")]


def ssh_checks(log):
    return [line for line in ssh_calls(log) if line.startswith("G")]


def wait_calls(log, count, timeout=10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if len(ssh_calls(log)) >= count:
            return True
        time.sleep(0.05)
    return len(ssh_calls(log)) >= count


def open_settings(client):
    client.send(SESSION + b"w")
    if not wait_text(client, "Workspaces"):
        return False
    client.send(b"c")
    return wait_text(client, "Settings")


def close_overlay(client, timeout=10.0):
    """Leave the settings screen and the list; Esc goes back one level at a time."""
    deadline = time.time() + timeout
    for _ in range(4):
        if "workspace list" not in status(client) and "[NORMAL]" in status(client):
            return True
        client.send(ESC)
        client.pump(0.2)
    return "workspace list" not in status(client) and "[NORMAL]" in status(client)


def state_bytes():
    try:
        with open(STATE, "rb") as handle:
            return handle.read()
    except OSError:
        return b""


def wait_state_version(version, timeout=20.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        data = state_bytes()
        if len(data) > 12 and data[:6] == MAGIC and data[6:7] == version:
            return True
        time.sleep(0.1)
    return False


def kill_server():
    pids = server_pids()
    for pid in pids:
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass
    deadline = time.time() + 5.0
    while time.time() < deadline and server_pids():
        time.sleep(0.05)
    return bool(pids) and not server_pids()


def scenario_settings_screen(res):
    begin("the workspace list opens the settings screen with c")
    env, _log = fake_ssh_env()
    client = Client(env=env)
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status(client))
        client.send(SESSION + b"w")
        res.check("the workspace list opens", wait_text(client, "Workspaces"))
        res.check("the list advertises the settings key",
                  "c settings" in client.screen.text(), status(client))
        client.send(b"c")
        res.check("c opens the settings screen", wait_text(client, "Settings"))
        res.check("the settings screen is named in the status bar",
                  wait_status(client, "[SETTINGS]"), status(client))
        screen = normalize_screen(client.screen.text())
        res.check("the settings title sits in the top border",
                  any("┌ Settings" in line for line in screen.splitlines()), screen)
        res.check("the alias row starts empty", "ssh alias: (none)" in client.screen.text())
        res.check("the footer lists the keys", "Enter edit" in client.screen.text()
                  and "Esc back" in client.screen.text())
        client.send(ARROW_DOWN)
        client.pump(0.2)
        client.send(ARROW_UP)
        res.check("the arrows keep the settings screen open",
                  wait_status(client, "[SETTINGS]") and "Settings" in client.screen.text(),
                  status(client))
        client.send(ESC)
        res.check("Esc goes back to the list",
                  wait_status(client, "workspace list"), status(client))
        res.check("the list is shown again", "Workspaces" in client.screen.text())
        client.send(ESC)
        res.check("Esc closes the list", wait_status(client, "[NORMAL] main"), status(client))
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def scenario_alias_panes(res):
    begin("an alias turns new panes into ssh panes; existing panes stay local")
    env, log = fake_ssh_env()
    client = Client(env=env)
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status(client))
        client.send(SESSION + b"w")
        wait_text(client, "Workspaces")
        client.send(b"n")
        res.check("a second workspace is created", wait_status(client, "[NORMAL] ws2"), status(client))
        client.send(ESC)
        res.check("the list closes", wait_status(client, "[NORMAL] ws2"), status(client))
        client.send("echo BEFORE-ALIAS\n")
        res.check("the workspace has a local pane", wait_text(client, "BEFORE-ALIAS"))

        res.check("the settings screen opens for the workspace", open_settings(client))
        client.send(ENTER)
        res.check("Enter starts the alias edit",
                  wait_status(client, "[ALIAS] ssh alias:"), status(client))
        client.send("myhost")
        res.check("the alias echoes while typing", wait_text(client, "alias: myhost_"))
        client.send(ENTER)
        res.check("the check result is shown",
                  wait_text(client, "ssh -G myhost: ok"), client.screen.text())
        res.check("the settings screen stays open", wait_status(client, "[SETTINGS]"), status(client))
        checks = ssh_checks(log)
        res.check("ssh -G ran once", len(checks) == 1, "calls=%r" % ssh_calls(log))
        res.check("ssh -G got the alias as one argument",
                  checks[:1] == ["G\t<-G>\t<myhost>"], "calls=%r" % ssh_calls(log))

        res.check("the settings screen and the list close", close_overlay(client), status(client))
        client.send("echo STILL-LOCAL\n")
        res.check("the pane made before the alias stays local", wait_text(client, "STILL-LOCAL"))
        res.check("the alias alone starts no ssh pane", ssh_runs(log) == [],
                  "calls=%r" % ssh_calls(log))

        client.send(PANE + b"n")
        res.check("a split starts an ssh pane", wait_text(client, "SSH-PANE:myhost"))
        runs = ssh_runs(log)
        res.check("the split passed the alias as one argument",
                  runs[:1] == ["RUN\t<myhost>"], "calls=%r" % ssh_calls(log))

        client.send(PANE + b"d")
        res.check("a second split starts its own ssh process",
                  wait_calls(log, 3) and len(ssh_runs(log)) == 2, "calls=%r" % ssh_calls(log))
        res.check("both ssh panes show their own connection",
                  pump_until(client, lambda c: c.screen.text().count("SSH-PANE:myhost") >= 2),
                  client.screen.text())
        res.check("the workspace keeps its name", "[NORMAL] ws2" in status(client), status(client))

        # A workspace created while an SSH workspace is active is a normal one.
        runs_before = len(ssh_runs(log))
        client.send(SESSION + b"w")
        res.check("the list opens from the ssh workspace", wait_text(client, "Workspaces"))
        client.send(b"n")
        created = wait_status(client, "[NORMAL] ws3")
        res.check("a workspace created from an ssh workspace opens",
                  created and close_overlay(client), status(client))
        client.send("echo WS-$((6*7))\n")
        res.check("the new workspace starts a local shell, not ssh",
                  wait_text(client, "WS-42"), client.screen.text())
        res.check("creating that workspace started no ssh process",
                  len(ssh_runs(log)) == runs_before, "runs=%r" % ssh_runs(log))
    finally:
        client.close()


def scenario_clear_alias(res):
    begin("clearing the alias turns the workspace back into a normal one")
    env, log = fake_ssh_env()
    client = Client(env=env)
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status(client))
        res.check("the settings screen opens", open_settings(client))
        client.send(ENTER)
        wait_status(client, "[ALIAS]")
        client.send("myhost")
        client.send(ENTER)
        res.check("the alias is set", wait_text(client, "ssh -G myhost: ok"), client.screen.text())
        close_overlay(client)
        client.send(PANE + b"n")
        res.check("a split runs ssh", wait_text(client, "SSH-PANE:myhost"))
        runs_before = len(ssh_runs(log))

        res.check("the settings screen opens again", open_settings(client))
        client.send(ENTER)
        res.check("the editor starts from the saved alias",
                  wait_text(client, "alias: myhost_"), client.screen.text())
        for _ in range(len("myhost")):
            client.send(BACKSPACE)
            client.pump(0.01)
        client.send(ENTER)
        res.check("the cleared alias is shown as empty",
                  wait_text(client, "ssh alias: (none)"), client.screen.text())
        res.check("clearing runs no ssh check", "ssh -G" not in client.screen.text(),
                  client.screen.text())
        close_overlay(client)

        client.send(PANE + b"n")
        client.send("echo LOCAL-AGAIN\n")
        res.check("a new split after clearing starts a local shell",
                  wait_text(client, "LOCAL-AGAIN"), client.screen.text())
        res.check("no ssh process was started after clearing",
                  len(ssh_runs(log)) == runs_before, "runs=%r" % ssh_runs(log))
    finally:
        client.close()


def scenario_odd_alias(res):
    begin("an alias with spaces and shell syntax stays one argument")
    env, log = fake_ssh_env()
    injected = os.path.join(RUNTIME_DIR, "INJECTED")
    alias = "my host; touch %s" % injected
    client = Client(env=env)
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status(client))
        res.check("the settings screen opens", open_settings(client))
        client.send(ENTER)
        wait_status(client, "[ALIAS]")
        client.send(alias)
        client.send(ENTER)
        res.check("the check runs for the odd alias",
                  wait_text(client, "ssh -G %s: ok" % alias), client.screen.text())
        res.check("the settings screen and the list close", close_overlay(client), status(client))

        client.send(PANE + b"n")
        res.check("a split starts the ssh pane",
                  wait_text(client, "SSH-PANE:") and wait_calls(log, 2),
                  "calls=%r" % ssh_calls(log))
        runs = ssh_runs(log)
        res.check("the alias reaches ssh as exactly one argument",
                  runs == ["RUN\t<%s>" % alias], "runs=%r" % runs)
        res.check("no shell interpreted the alias", not os.path.exists(injected),
                  "created=%s" % injected)
    finally:
        client.close()


def scenario_check_failure(res):
    begin("a failed ssh -G is reported and the alias is still saved")
    env, log = fake_ssh_env()
    client = Client(env=env)
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status(client))
        res.check("the settings screen opens", open_settings(client))
        client.send(ENTER)
        wait_status(client, "[ALIAS]")
        client.send("missing-alias")
        client.send(ENTER)
        res.check("the failure is reported",
                  wait_text(client, "ssh -G missing-alias: failed"), client.screen.text())
        res.check("the settings screen stays open", wait_status(client, "[SETTINGS]"), status(client))
        res.check("the check ran once", len(ssh_checks(log)) == 1, "calls=%r" % ssh_calls(log))
        res.check("the settings screen and the list close", close_overlay(client), status(client))

        res.check("the settings screen opens again", open_settings(client))
        res.check("the alias was saved despite the failure",
                  "ssh alias: missing-alias" in client.screen.text(), client.screen.text())
        res.check("the settings screen and the list close again", close_overlay(client))
    finally:
        client.close()


def scenario_restore(res):
    begin("a restart keeps the alias and the definition but starts no ssh")
    env, log = fake_ssh_env()
    client = Client(env=env)
    try:
        res.check("the session starts", wait_status(client, "[NORMAL] main"), status(client))
        res.check("the settings screen opens", open_settings(client))
        client.send(ENTER)
        wait_status(client, "[ALIAS]")
        client.send("myhost")
        client.send(ENTER)
        wait_text(client, "ssh -G myhost: ok")
        close_overlay(client)
        client.send(PANE + b"n")
        res.check("an ssh pane is running", wait_text(client, "SSH-PANE:myhost"))
        runs_before = len(ssh_runs(log))

        since = time.time()
        client.detach_keys()
        client.wait_exit(10)
    finally:
        client.close()

    deadline = time.time() + 15
    while time.time() < deadline:
        try:
            if os.stat(STATE).st_mtime >= since and os.stat(STATE).st_size > 0:
                break
        except OSError:
            pass
        time.sleep(0.1)
    res.check("the state file was written", os.path.exists(STATE))
    res.check("the state file is format 2", state_bytes()[6:7] == FORMAT_V2,
              "head=%r" % state_bytes()[:8])
    res.check("the server is stopped", kill_server())

    second = Client(env=env)
    try:
        res.check("the restarted server restores the workspace",
                  wait_status(second, "[NORMAL] main"), status(second))
        second.send("echo AFTER-RESTART\n")
        res.check("the restored pane is a local shell",
                  wait_text(second, "AFTER-RESTART"))
        res.check("no ssh connection was resumed",
                  len(ssh_runs(log)) == runs_before, "runs=%r" % ssh_runs(log))
        res.check("the settings screen opens after the restart", open_settings(second))
        res.check("the alias came back", "ssh alias: myhost" in second.screen.text(),
                  second.screen.text())
        close_overlay(second)
        second.send("exit\n")
        second.wait_exit(10)
    finally:
        second.close()


def scenario_migrate(res):
    begin("a format-1 state file is read as format 2 without losing workspaces")
    env, _log = fake_ssh_env()
    shutil.copy(FIXTURE, STATE)
    client = Client(env=env)
    try:
        res.check("the saved state is accepted",
                  b"saved state was unreadable" not in client.raw)
        res.check("the first workspace and its renamed tab come back",
                  wait_status(client, "[NORMAL] main | 1:logs*"), status(client))
        client.send(SESSION + b"w")
        res.check("both workspaces are listed", wait_text(client, "Workspaces"))
        screen = normalize_screen(client.screen.text())
        res.check("the saved workspace names are kept",
                  "main " in screen and "ws2 " in screen, screen)
        client.send(b"c")
        res.check("the migrated workspace opens its settings",
                  wait_text(client, "Settings"))
        res.check("a migrated workspace has no alias",
                  "ssh alias: (none)" in client.screen.text(), client.screen.text())
        res.check("the state file is rewritten as format 2", wait_state_version(FORMAT_V2),
                  "head=%r" % state_bytes()[:8])
        close_overlay(client)
        client.send("exit\n")
        client.wait_exit(10)
    finally:
        client.close()


def main():
    res = Result()
    try:
        scenario_settings_screen(res)
        scenario_alias_panes(res)
        scenario_clear_alias(res)
        scenario_odd_alias(res)
        scenario_check_failure(res)
        scenario_restore(res)
        scenario_migrate(res)
    finally:
        cleanup()
    return res.report("ssh workspace")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
