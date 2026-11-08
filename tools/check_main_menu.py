#!/usr/bin/env python3
"""Drive the rewrite main menu under Xvfb and audit its private screenshots."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time

from PIL import Image, ImageStat


# Each case uses ordinary X11 device events, not a test-only menu dispatcher.
CASES = {
    "main": ([], 0, None, False, None),
    "attached": (["Return"], 0, "Store", False, None),
    "adventure": (["Return", "Down", "Return"], 1, "Adventure", False, None),
    "online": (["Return", "Down", "Right", "Down", "Return"], 5, "Online", False, None),
    "options": (["Return", "Down", "Right", "Down", "Down", "Right", "Return"], 6, "Options", False, None),
    "quit": (["Return", "Down", "Down", "Down", "Return"], 0, "Exit", True, None),
    "cancel_quit": (["Return", "Down", "Down", "Down", "Return", "Escape"], 0, "Exit", False, None),
    "dlc": (["Return", "Down", "Right", "Return"], 0, "DLC", False, "DLC"),
    "pointer_options": (["click_options"], 6, "Options", False, None),
    "pointer_overlap": (["overlap"], 0, "Online", False, None),
    "pointer_cancel": (["cancel_click"], 0, None, False, None),
    "decline_quit": (["Return", "Down", "Down", "Down", "Return", "Right", "Return"], 0, "Exit", False, None),
    "quit_yes": (["Return", "Down", "Down", "Down", "Return", "Return"], None, None, False, None),
}


def xdo(*args):
    return subprocess.check_output(["xdotool", *map(str, args)], text=True, timeout=10).strip()


def drive(viewer: Path, content: Path, output: Path):
    output.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    # Xvfb owns DISPLAY; inherited Wayland variables otherwise let Winit
    # create the window on the desktop instead of the isolated test display.
    env.pop("WAYLAND_DISPLAY", None)
    env.pop("WAYLAND_SOCKET", None)
    for name, (inputs, *_expected) in CASES.items():
        with (output / f"{name}.log").open("w") as log:
            command = [
                str(viewer.resolve()), "--menu", str(content.resolve()),
                "--root", str((output / "user-cache").resolve()),
            ]
            if name != "quit_yes":
                command += ["--snapshot", str((output / f"{name}.png").resolve()), "--snapshot-after", "4"]
            proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, start_new_session=True, env=env)
            try:
                window = xdo("search", "--sync", "--pid", proc.pid, "--name", "Magicka rewrite").splitlines()[0]
                xdo("windowfocus", "--sync", window)
                xdo("mousemove", "--window", window, "--sync", 0, 0)
                time.sleep(0.6)  # Allow the first rendered/input frame to settle.
                assert proc.poll() is None, f"{name}: viewer exited before input"
                for key in inputs:
                    if key == "click_options":
                        xdo("mousemove", "--window", window, "--sync", 743, 667)
                        xdo("click", 1)
                    elif key == "overlap":
                        xdo("mousemove", "--window", window, "--sync", 577, 422)
                    elif key == "cancel_click":
                        xdo("mousemove", "--window", window, "--sync", 743, 667)
                        xdo("mousedown", 1)
                        time.sleep(0.12)
                        xdo("mousemove", "--window", window, "--sync", 0, 0)
                        xdo("mouseup", 1)
                    else:
                        xdo("key", "--clearmodifiers", key)
                    time.sleep(0.12)
                assert proc.wait(timeout=5 if name == "quit_yes" else 25) == 0, f"{name}: viewer failed"
                if name == "quit_yes":
                    (output / "quit_yes.json").write_text(json.dumps({"exit_code": proc.returncode, "snapshot_timer": False}))
            finally:
                if proc.poll() is None:
                    os.killpg(proc.pid, signal.SIGTERM)
                    try:
                        proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(proc.pid, signal.SIGKILL)
                        proc.wait()


def audit(output: Path):
    hashes = {}
    for name, (_, page, selected, quit_dialog, notice) in CASES.items():
        state = json.loads((output / f"{name}.json").read_text())
        if name == "quit_yes":
            assert state == {"exit_code": 0, "snapshot_timer": False}, state
            continue
        assert state["page"] == page, (name, state)
        assert state["selected"] == selected, (name, state)
        assert state["quit_dialog"] == quit_dialog, (name, state)
        assert state["notice"] == notice, (name, state)
        image = Image.open(output / f"{name}.png").convert("RGB")
        assert image.size == (960, 720), (name, image.size)
        assert max(ImageStat.Stat(image).stddev) > 10, f"{name}: blank render"
        hashes[name] = hashlib.sha256(image.tobytes()).hexdigest()
    assert hashes["main"] != hashes["attached"], "Store focus did not change its rendered tile"
    assert hashes["quit"] != hashes["cancel_quit"], "Quit cancellation did not remove its question"
    assert hashes["main"] != hashes["pointer_overlap"], "Pointer focus did not highlight the overlapping card"
    assert hashes["main"] == hashes["pointer_cancel"], "Cancelled click changed the main page"
    return {"captures": len(hashes), "scenarios":len(CASES), "device_input": True, "quit_yes_exit": True, "hashes": hashes}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--drive", action="store_true")
    parser.add_argument("--viewer", type=Path, default=Path("output/rewrite/target/release/magicka-viewer"))
    parser.add_argument("--content", type=Path, default=Path("target/Magicka/Content"))
    args = parser.parse_args()
    if args.drive:
        drive(args.viewer, args.content, args.output)
    print(json.dumps(audit(args.output), indent=2))
