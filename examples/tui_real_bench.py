#!/usr/bin/env python3
"""Real-process cold start + resident memory, per transcript scale.

Two modes, each in its own isolated XDG root so nothing touches the user's
real state (DB, socket, config):

  draft <n> ...   `mypi` in the draft state (no session): binary + config +
                  daemon handshake + first paint. This is the "cold start"
                  a user actually feels when they launch the TUI.
  attach <n> ...  `mypi attach 1` against a stored session of n entries:
                  the same, plus the daemon's snapshot load and the front
                  end's transcript ingest.

Reported per run: wall time to the first painted byte, TUI VmRSS/VmHWM, and
the daemon's VmRSS/VmHWM once the attach snapshot has landed.
"""
import fcntl
import os
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import termios
import time

BIN = "./target/release/mypi"
GEN = "./target/release/examples/tui_scale_bench"
ROOT = "/tmp/mypi-scale"
COLS, ROWS = 120, 40


def rss_kb(pid):
    try:
        with open(f"/proc/{pid}/status") as f:
            s = f.read()
    except OSError:
        return 0, 0

    def g(k):
        m = re.search(rf"^{k}:\s+(\d+)", s, re.M)
        return int(m.group(1)) if m else 0

    return g("VmRSS"), g("VmHWM")


def boot(n):
    """Isolated data/socket root + a generated session of n entries."""
    data_home = f"{ROOT}/xdg-{n}"
    rt = f"{ROOT}/rt-{n}"
    os.makedirs(f"{data_home}/mypi", exist_ok=True)
    os.makedirs(rt, exist_ok=True)
    db = f"{data_home}/mypi/sessions.db3"
    if not os.path.exists(db):
        subprocess.run([GEN, "gen", str(n), db], check=True, capture_output=True)
    env = dict(os.environ)
    env["XDG_DATA_HOME"] = data_home
    env["XDG_RUNTIME_DIR"] = rt
    env["TERM"] = "xterm-256color"
    env.pop("MYPI_CONFIG", None)
    return env, rt


def start_daemon(env, rt):
    d = subprocess.Popen(
        [BIN, "--server"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    for _ in range(200):
        if os.path.exists(f"{rt}/mypi.sock"):
            break
        time.sleep(0.05)
    time.sleep(0.3)
    return d


def run_tui(env, argv, watch_secs, stop_after_paint=None):
    t0 = time.time()
    pid, fd = pty.fork()
    if pid == 0:
        os.environ.update(env)
        os.execv(BIN, [BIN, *argv])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    first = None
    paint = None
    content = None
    total = 0
    seen_alt = False
    rss_peak = 0
    rss_last = 0
    deadline = t0 + watch_secs
    while time.time() < deadline:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                b = os.read(fd, 1 << 20)
            except OSError:
                break
            total += len(b)
            # When the TUI had to spawn the daemon, the daemon's own banner
            # ("mypi daemon listening on ...") arrives on this same pty first —
            # it inherits stdout. The frame starts at the alt-screen switch.
            if not seen_alt and b"\x1b[?1049h" in b:
                seen_alt = True
                paint = time.time() - t0
            if b and first is None:
                # The first frame is the statusline: it paints before the
                # transcript has even been decoded. The number a user cares
                # about is when the *conversation* appears.
                first = time.time() - t0
            if content is None and total > 3000:
                content = time.time() - t0
            while True:
                r2, _, _ = select.select([fd], [], [], 0)
                if not r2:
                    break
                try:
                    if not os.read(fd, 1 << 20):
                        break
                except OSError:
                    break
        rss, hwm = rss_kb(pid)
        rss_last = max(rss_last, rss)
        rss_peak = max(rss_peak, hwm)
    rss, hwm = rss_kb(pid)
    rss_last = max(rss_last, rss)
    rss_peak = max(rss_peak, hwm)
    try:
        os.kill(pid, signal.SIGKILL)
    except OSError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    return first, paint, content, rss_last, rss_peak


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "draft"
    scales = [int(a) for a in sys.argv[2:]] or [8000]
    for n in scales:
        env, rt = boot(n)
        if mode == "cold":
            # Cold start *from the server*: nothing is running, so the TUI
            # spawns the daemon itself and waits for it to answer. This is
            # what a user's first `mypi` of the day costs.
            subprocess.run(["pkill", "-f", "mypi --server"], capture_output=True)
            time.sleep(0.3)
            first, paint, _, rss, peak = run_tui(env, [], watch_secs=6.0)
            print(
                f"cold   n={n:>6}  exec->daemon-banner {first*1e3 if first else -1:>6.1f} ms  "
                f"exec->first frame {paint*1e3 if paint else -1:>6.1f} ms  "
                f"tui rss {rss/1024:>6.1f} MB peak {peak/1024:>6.1f} MB",
                flush=True,
            )
            subprocess.run(["pkill", "-f", "mypi --server"], capture_output=True)
            continue
        daemon = start_daemon(env, rt)
        if mode == "draft":
            for run in range(3):
                first, _, _, rss, peak = run_tui(env, [], watch_secs=4.0)
                print(
                    f"draft  n={n:>6} run{run}  cold-start {first*1e3 if first else -1:>7.1f} ms  "
                    f"tui rss {rss/1024:>6.1f} MB  peak {peak/1024:>6.1f} MB",
                    flush=True,
                )
        else:
            first, paint, content, rss, peak = run_tui(
                env, ["attach", "1"], watch_secs=8.0
            )
            d_rss, d_hwm = rss_kb(daemon.pid)
            print(
                f"attach n={n:>6}  statusline {first if first else -1:>6.2f} s  "
                f"transcript {content if content else -1:>6.2f} s  "
                f"tui rss {rss/1024:>6.1f} MB peak {peak/1024:>6.1f} MB  "
                f"daemon rss {d_rss/1024:>6.1f} MB peak {d_hwm/1024:>6.1f} MB",
                flush=True,
            )
        daemon.terminate()
        try:
            daemon.wait(timeout=5)
        except subprocess.TimeoutExpired:
            daemon.kill()


if __name__ == "__main__":
    main()
