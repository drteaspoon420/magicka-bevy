"""Exercise QA deadlines and cleanup with TERM-resistant parent/child jobs."""
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[3]
STALLED = 'trap "" TERM; sleep 300 & printf "%s %s" "$$" "$!" > "$1"; wait'


def running(pid):
    try:
        return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0] != "Z"
    except FileNotFoundError:
        return False


def check(mode, directory):
    pidfile = directory / f"{mode}.pids"
    if mode == "deadline":
        command = ["timeout", "--kill-after=1s", "1s", "bash", "-c", STALLED, "qa", str(pidfile)]
    else:
        command = ["bash", "-c", '''
set -euo pipefail
source "$1"
timeout --kill-after=10s 300s bash -c "$2" qa "$3" &
qa_pid=$!
trap 'qa_stop_processes "$qa_pid"' EXIT
for ((attempt=0;attempt<100;attempt++)); do
  [[ -s "$3" ]] && break
  sleep 0.02
done
[[ -s "$3" ]]
exit 23
''', "qa", str(ROOT / "bin/network-qa-process.sh"), STALLED, str(pidfile)]
    started = time.monotonic()
    # This outer deadline makes failure of the deadline mechanism fail the check.
    process = subprocess.Popen(command, start_new_session=True)
    pids = []
    try:
        status = process.wait(timeout=12)
        pids = [int(pid) for pid in pidfile.read_text().split()]
        for _ in range(100):
            if not any(running(pid) for pid in pids):
                break
            time.sleep(0.02)
        assert len(pids) == 2 and not any(running(pid) for pid in pids), f"{mode}: stalled job survived"
        expected = (124, -signal.SIGKILL, 137) if mode == "deadline" else (23,)
        assert status in expected, f"{mode}: unexpected status {status}"
        print(f"PASS {mode}: stalled parent and child stopped in {time.monotonic() - started:.2f}s", flush=True)
    finally:
        if pidfile.exists():
            pids = [int(pid) for pid in pidfile.read_text().split()]
        for pid in pids:
            if running(pid):
                os.kill(pid, signal.SIGKILL)
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=2)


with tempfile.TemporaryDirectory(prefix="magicka-qa-timeout-") as temporary:
    for mode in ("deadline", "cleanup"):
        check(mode, Path(temporary))
