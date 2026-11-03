#!/usr/bin/env python3
"""Server control: `koshi kill` and recovery from a stale socket."""

import os
import signal
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    BIN,
    RUNTIME_DIR,
    SOCKET_PATH,
    Client,
    Result,
    clean_runtime,
    cleanup,
    pump_until,
    server_alive,
    server_pids,
    status_line,
)


def begin(name):
    cleanup()
    clean_runtime()
    print("scenario: %s" % name)


def run_kill():
    env = dict(os.environ)
    env["KOSHI_RUNTIME_DIR"] = RUNTIME_DIR
    return subprocess.run([BIN, "kill"], capture_output=True, text=True, timeout=20, env=env)


def wait_gone(timeout=10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if not server_alive():
            return True
        time.sleep(0.1)
    return False


def scenario_kill(res):
    begin("koshi kill stops the server and saves the state")
    client = Client()
    try:
        res.check("the session is up", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)), status_line(client))
        client.send("echo KILL-ME-NOT-ON-SCREEN\\n")
        res.check("the pane works", pump_until(
            client, lambda c: "KILL-ME-NOT-ON-SCREEN" in c.screen.text()))
        client.detach_keys()
        res.check("the client detaches", client.wait_exit(10) == 0)
        res.check("the server survives the detach", server_alive())
    finally:
        client.close()

    pids = server_pids()
    res.check("the server pid is known", bool(pids), "pids=%r" % pids)
    result = run_kill()
    res.check("koshi kill exits 0", result.returncode == 0,
              "stdout=%r stderr=%r" % (result.stdout, result.stderr))
    res.check("koshi kill names the pid", bool(pids) and str(pids[0]) in result.stdout,
              "stdout=%r" % result.stdout)
    res.check("the server is gone", wait_gone())
    res.check("the socket is removed", not os.path.exists(SOCKET_PATH),
              "socket=%s" % SOCKET_PATH)

    again = run_kill()
    res.check("killing again reports no server", again.returncode != 0,
              "exit=%d stdout=%r" % (again.returncode, again.stdout))
    res.check("the second kill says nothing is running", "no server" in again.stdout,
              "stdout=%r" % again.stdout)


def scenario_stale_socket(res):
    begin("a socket left by a killed server is cleaned up")
    client = Client()
    try:
        res.check("the session is up", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)), status_line(client))
        client.detach_keys()
        client.wait_exit(10)
    finally:
        client.close()

    pids = server_pids()
    res.check("the server is running", bool(pids))
    for pid in pids:
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass
    deadline = time.time() + 5
    while time.time() < deadline and server_pids():
        time.sleep(0.05)
    res.check("the server was killed abruptly", not server_pids())
    res.check("the socket file is still there", os.path.exists(SOCKET_PATH))

    result = run_kill()
    res.check("killing a dead server reports no server", result.returncode != 0,
              "exit=%d stdout=%r" % (result.returncode, result.stdout))
    res.check("the stale socket is removed", not os.path.exists(SOCKET_PATH),
              "socket=%s" % SOCKET_PATH)

    again = Client()
    try:
        res.check("a new session starts after the cleanup", pump_until(
            again, lambda c: "[NORMAL] main" in status_line(c)), status_line(again))
        again.send("echo FRESH-AFTER-STALE\\n")
        res.check("the new session works", pump_until(
            again, lambda c: "FRESH-AFTER-STALE" in c.screen.text()))
        again.detach_keys()
        res.check("the new client detaches", again.wait_exit(10) == 0)
    finally:
        again.close()
    run_kill()


def main():
    res = Result()
    try:
        scenario_kill(res)
        scenario_stale_socket(res)
    finally:
        cleanup()
    return res.report("server control: kill and stale socket recovery")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
