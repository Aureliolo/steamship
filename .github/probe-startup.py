"""Hold steamcmd at its prompt on a pipe and time when its log files change.

Usage: python probe-startup.py <steamship home> <windows|linux|macos> <seconds>
"""

import os
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

home = Path(sys.argv[1])
platform = sys.argv[2]
hold = float(sys.argv[3])
root = home / "steamcmd"
if platform.startswith("windows"):
    program, state, env = root / "steamcmd.exe", root, dict(os.environ)
elif platform == "linux":
    program, state, env = root / "steamcmd.sh", home / "Steam", dict(os.environ, HOME=str(home))
else:
    state = home / "Library" / "Application Support" / "Steam"
    program, env = root / "steamcmd.sh", dict(os.environ, HOME=str(home))
logs = state / "logs"
args = [str(program), "+@ShutdownOnFailedCommand", "1", "+@NoPromptForPassword", "1",
        "+login", "anonymous"]
start = time.monotonic()
extra = {"creationflags": 0x08000000} if os.name == "nt" else {"start_new_session": True}
child = subprocess.Popen(args, cwd=root, env=env, stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT, **extra)


def read():
    for line in iter(child.stdout.readline, b""):
        text = line.decode(errors="replace").rstrip()
        print(f"{time.monotonic() - start:6.2f} pipe  {text[:90]}", flush=True)


threading.Thread(target=read, daemon=True).start()
sizes = {}
while time.monotonic() - start < hold:
    for name in ("connection_log.txt", "console_log.txt", "bootstrap_log.txt"):
        path = logs / name
        size = path.stat().st_size if path.exists() else -1
        if sizes.get(name) != size:
            sizes[name] = size
            alive = child.poll() is None
            print(f"{time.monotonic() - start:6.2f} file  {name} {size} bytes, running {alive}",
                  flush=True)
    time.sleep(0.05)
print(f"{time.monotonic() - start:6.2f} end   exit so far {child.poll()}", flush=True)
if os.name == "nt":
    child.kill()
else:
    os.killpg(child.pid, signal.SIGKILL)
child.wait()
