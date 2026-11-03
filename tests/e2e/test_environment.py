#!/usr/bin/env python3
"""M4 acceptance: the environment the multiplexer has to survive.

Scenario 1 puts a 200 ms Unix-socket proxy between the client and the server and
checks that input still arrives and that the screen stays consistent.
Scenario 2 kills that proxy to drop the connection (the same path an SSH drop
takes) and checks that the server, the pane process, and the screen survive.
Scenario 3 asks the kernel whether koshi listens on TCP or connects out.
Scenario 4 runs with no configuration, no state, and default XDG locations.
"""

import os
import re
import shutil
import socket
import stat
import subprocess
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from harness import (  # noqa: E402
    pane_state,
    ROWS,
    RUNTIME_DIR,
    SOCKET_PATH,
    Client,
    Result,
    clean_runtime,
    cleanup,
    osc52,
    pump_until,
    send_mouse,
    server_alive,
    status_line,
)

PROXY_DIR = os.path.join(RUNTIME_DIR, "proxy")
PROXY_SOCKET = os.path.join(PROXY_DIR, "koshi.sock")
DELAY = 0.2


def begin(name):
    cleanup()
    clean_runtime()
    print("scenario: %s" % name)


class Proxy:
    """Forward one Unix socket to another, delaying every chunk by `delay`."""

    def __init__(self, listen, target, delay=DELAY):
        self.listen_path = listen
        self.target = target
        self.delay = delay
        self.pairs = []
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        os.makedirs(os.path.dirname(listen), mode=0o700, exist_ok=True)
        try:
            os.unlink(listen)
        except OSError:
            pass
        self.listener.bind(listen)
        os.chmod(listen, 0o600)
        self.listener.listen(8)
        self.stopped = False
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self):
        while not self.stopped:
            try:
                client, _ = self.listener.accept()
            except OSError:
                return
            try:
                server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                server.connect(self.target)
            except OSError:
                client.close()
                continue
            self.pairs.append((client, server))
            threading.Thread(target=self._pump, args=(client, server), daemon=True).start()
            threading.Thread(target=self._pump, args=(server, client), daemon=True).start()

    def _pump(self, source, sink):
        while True:
            try:
                data = source.recv(65536)
            except OSError:
                return
            if not data:
                return
            time.sleep(self.delay)
            try:
                sink.sendall(data)
            except OSError:
                return

    def kill(self):
        """Drop every connection, like a link that dies mid-session."""
        self.stopped = True
        try:
            self.listener.close()
        except OSError:
            pass
        for pair in self.pairs:
            for sock in pair:
                try:
                    sock.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                try:
                    sock.close()
                except OSError:
                    pass
        self.pairs = []
        try:
            os.unlink(self.listen_path)
        except OSError:
            pass


def pid_alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def pid_from(screen, marker):
    for line in screen.splitlines():
        if marker in line:
            tail = line.split(marker)[-1].strip()
            digits = "".join(ch for ch in tail if ch.isdigit())
            if digits:
                return int(digits)
    return 0


def scenario_latency(res):
    begin("200 ms each way: input still reaches the pane and the screen stays whole")
    base = Client()
    try:
        res.check("the server is up", pump_until(
            base, lambda c: "[NORMAL] main" in status_line(c)), status_line(base))
        base.send("echo BASE-READY\n")
        res.check("the pane works before the proxy", pump_until(
            base, lambda c: "BASE-READY" in c.screen.text()))
        base.detach_keys()
        res.check("the direct client detaches", base.wait_exit(10) == 0)
    finally:
        base.close()

    proxy = Proxy(PROXY_SOCKET, SOCKET_PATH)
    slow = Client(env={"KOSHI_RUNTIME_DIR": PROXY_DIR})
    try:
        res.check("the delayed client attaches through the proxy", pump_until(
            slow, lambda c: "[NORMAL] main" in status_line(c)), status_line(slow))

        started = time.time()
        slow.send("echo DELAYED-OK\n")
        res.check("typed input reaches the pane over the delayed link", pump_until(
            slow, lambda c: "DELAYED-OK" in c.screen.text()), status_line(slow))
        elapsed = time.time() - started
        res.check("the proxy really delayed the round trip", elapsed >= DELAY,
                  "elapsed=%.2fs" % elapsed)
        res.check("the status bar is intact after the delayed echo",
                  pane_state(slow) == (1, 1), status_line(slow))

        slow.send("for i in $(seq -w 1 100); do echo SLOW-$i; done\n")
        res.check("a burst of output is not lost", pump_until(
            slow, lambda c: "SLOW-100" in c.screen.text(), 30))
        first = find_text(slow, "SLOW-099")[0]
        last = find_text(slow, "SLOW-100")[0]
        res.check("the newest lines arrived in order",
                  first is not None and last is not None and last == first + 1,
                  "rows=%r,%r" % (first, last))
        res.check("the burst did not garble the status bar",
                  "[NORMAL] main" in status_line(slow), status_line(slow))
        slow.detach_keys()
        res.check("the delayed client detaches", slow.wait_exit(20) == 0)
    finally:
        slow.close()
        proxy.kill()


def scenario_disconnect(res):
    begin("dropping the link keeps the server and the pane, reattaching matches")
    proxy = None
    slow = None
    direct = Client()
    try:
        res.check("the server is up", pump_until(
            direct, lambda c: "[NORMAL] main" in status_line(c)), status_line(direct))
        direct.send("sleep 300 & echo DISCONNECT-SLEEPER=$!\n")
        screen = direct.wait_for("DISCONNECT-SLEEPER=", 10) or ""
        sleeper = pid_from(screen, "DISCONNECT-SLEEPER=")
        res.check("a long running pane process started", sleeper > 0, "pid=%s" % sleeper)
        direct.send("echo KEPT-MARK\n")
        res.check("the pane printed a marker", pump_until(
            direct, lambda c: "KEPT-MARK" in c.screen.text()))
        direct.detach_keys()
        res.check("the direct client detaches", direct.wait_exit(10) == 0)

        proxy = Proxy(PROXY_SOCKET, SOCKET_PATH)
        slow = Client(env={"KOSHI_RUNTIME_DIR": PROXY_DIR})
        res.check("the remote client attaches", pump_until(
            slow, lambda c: "KEPT-MARK" in c.screen.text()), status_line(slow))
        before = slow.stable_screen()

        proxy.kill()
        code = slow.wait_exit(15)
        res.check("the client on the dropped link exits", code is not None, "exit=%r" % code)
        res.check("the server survived the drop", server_alive())
        res.check("the pane process survived the drop", pid_alive(sleeper), "pid=%s" % sleeper)

        back = Client()
        try:
            res.check("a direct client can attach again", pump_until(
                back, lambda c: "[NORMAL] main" in status_line(c)), status_line(back))
            after = back.stable_screen()
            res.check("the reattached screen matches the dropped one", after == before,
                      "before=%r after=%r" % (before.split("\n")[-1:], after.split("\n")[-1:]))
            back.send("echo AFTER-RECONNECT\n")
            res.check("the reattached pane accepts input", pump_until(
                back, lambda c: "AFTER-RECONNECT" in c.screen.text()))
        finally:
            back.close()
    finally:
        if slow is not None:
            slow.close()
        if proxy is not None:
            proxy.kill()
        direct.close()


def scenario_listening(res):
    begin("no TCP listener and no outgoing connection, one Unix socket")
    client = Client()
    try:
        res.check("the server is up", pump_until(
            client, lambda c: "[NORMAL] main" in status_line(c)), status_line(client))
        res.check("the server process is alive", server_alive())
        if not shutil.which("ss"):
            print("  SKIP ss is not installed, the socket inventory was not checked")
            return
        listening = subprocess.run(["ss", "-ltnp"], capture_output=True, text=True).stdout
        connections = subprocess.run(["ss", "-tnp"], capture_output=True, text=True).stdout
        unix = subprocess.run(["ss", "-lx"], capture_output=True, text=True).stdout

        res.check("koshi listens on no TCP port", "koshi" not in listening,
                  listening.strip().split("\n")[-1] if listening.strip() else "")
        res.check("koshi opens no TCP connection", "koshi" not in connections,
                  connections.strip().split("\n")[-1] if connections.strip() else "")
        res.check("the Unix socket is the only endpoint", SOCKET_PATH in unix,
                  "socket=%s" % SOCKET_PATH)
        res.check("the socket is not a TCP port", ":2" not in unix and not re.search(
            r"koshi.*\d+\.\d+\.\d+\.\d+", unix))
    finally:
        client.close()


def find_text(client, needle):
    for row in range(ROWS):
        col = client.screen.line(row).find(needle)
        if col >= 0:
            return row, col
    return None, None


def scenario_defaults(res):
    begin("no configuration, no state, default XDG locations")
    home = os.path.join(RUNTIME_DIR, "home")
    xdg_config = os.path.join(RUNTIME_DIR, "xdg-config")
    xdg_state = os.path.join(RUNTIME_DIR, "xdg-state")
    xdg_runtime = os.path.join(RUNTIME_DIR, "xdg-runtime")
    for path in (home, xdg_config, xdg_state, xdg_runtime):
        os.makedirs(path, exist_ok=True)
    env = {
        "HOME": home,
        "XDG_CONFIG_HOME": xdg_config,
        "XDG_STATE_HOME": xdg_state,
        "XDG_RUNTIME_DIR": xdg_runtime,
        # An empty value makes the server fall back to the XDG defaults.
        "KOSHI_RUNTIME_DIR": "",
    }
    client = Client(env=env)
    try:
        res.check("the default session shows the normal state",
                  pump_until(client, lambda c: "[NORMAL] main" in status_line(c)),
                  status_line(client))
        res.check("the default session has one tab and one pane",
                  "1:shell*" in status_line(client) and pane_state(client) == (1, 1),
                  status_line(client))
        client.send(b"\x10")
        res.check("the default leader is Ctrl+p", pump_until(
            client, lambda c: "[PANE]" in status_line(c)), status_line(client))
        client.send(b"\x1b")
        res.check("escape cancels the leader", pump_until(
            client, lambda c: "[NORMAL]" in status_line(c)), status_line(client))

        client.send(b"\x10r")
        res.check("a split works without a configuration file", pump_until(
            client, lambda c: pane_state(c) == (2, 2)), status_line(client))
        client.send("exit\n")
        res.check("closing the split pane works", pump_until(
            client, lambda c: pane_state(c) == (1, 1)), status_line(client))

        client.send("echo COPY-ME-42\n")
        res.check("the pane prints a marker", pump_until(
            client, lambda c: "COPY-ME-42" in c.screen.text()))
        client.wait_quiet(quiet=0.2)
        row, col = find_text(client, "COPY-ME-42")
        res.check("the marker row was found", row is not None, "row=%r" % row)
        if row is not None:
            send_mouse(client, 0, col, row)
            send_mouse(client, 32, col + 10, row)
            send_mouse(client, 32, col + 10, row, release=True)
            res.check("dragging and releasing copies over OSC 52", pump_until(
                client, lambda c: any("COPY-ME-42" in text for text in osc52(c))),
                "payloads=%r" % osc52(client))

        client.send("exit\n")
        res.check("the client exits when the default shell exits",
                  client.wait_exit(10) is not None)
    finally:
        client.close()

    state = os.path.join(xdg_state, "koshi", "state.bin")
    deadline = time.time() + 10.0
    while time.time() < deadline and not os.path.exists(state):
        time.sleep(0.05)
    res.check("the state file lands in XDG_STATE_HOME", os.path.exists(state),
              "path=%s" % state)
    res.check("no state file is written to the runtime directory",
              not os.path.exists(os.path.join(RUNTIME_DIR, "state.bin")))
    if os.path.exists(state):
        dir_mode = stat.S_IMODE(os.stat(os.path.dirname(state)).st_mode)
        file_mode = stat.S_IMODE(os.stat(state).st_mode)
        res.check("the default state directory is 0700", dir_mode == 0o700, "mode=%o" % dir_mode)
        res.check("the default state file is 0600", file_mode == 0o600, "mode=%o" % file_mode)
    res.check("no configuration file was needed", not os.path.exists(
        os.path.join(xdg_config, "koshi", "config.toml")))


def main():
    res = Result()
    try:
        scenario_latency(res)
        scenario_disconnect(res)
        scenario_listening(res)
        scenario_defaults(res)
    finally:
        cleanup()
    return res.report("environment: latency, disconnect, sockets, defaults")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
