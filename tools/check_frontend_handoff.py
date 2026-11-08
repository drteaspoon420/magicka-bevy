#!/usr/bin/env python3
"""Drive menu -> opening -> loading -> gameplay -> menu -> second session -> menu
in one process and one X11 window, using real X11 input."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import time

from PIL import Image, ImageStat


def xdo(*args):
    return subprocess.check_output(["xdotool", *map(str, args)], text=True, timeout=10).strip()


def wait_until(predicate, seconds, message):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        result = predicate()
        if result:
            return result
        time.sleep(0.2)
    raise AssertionError(message)


def run(viewer, content, cache, output, skipped):
    output.mkdir(parents=True, exist_ok=True)
    for name in ["opening.json", "loading.json", "host.json", "result.json", "loading.png", "playing.png",
                 "returned.png", "session1-loading.json", "session1-playing.png"]:
        (output / name).unlink(missing_ok=True)
    user = output / "user"
    user.mkdir(exist_ok=True)
    for name in ["gltf", "textures"]:
        asset_dir = user / name
        if not asset_dir.exists():
            asset_dir.symlink_to((cache / name).resolve(), target_is_directory=True)
    trace = output / "opening.json"
    host = output / "host.json"
    env = os.environ.copy()
    env.pop("WAYLAND_DISPLAY", None)
    env.pop("WAYLAND_SOCKET", None)
    command = [str(viewer.resolve()), "--menu", str(content.resolve()), "--root", str(user.resolve()),
               "--menu-page", "9", "--silent", "--opening-trace", str(trace.resolve()),
               "--loading-audit", str(output.resolve()), "--menu-audit-repeat", "1",
               "--screenshots", str(output.resolve())]
    with (output / "viewer.log").open("w") as log:
        proc = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            window = xdo("search", "--sync", "--all", "--pid", proc.pid, "--name", "^Magicka rewrite$").splitlines()[0]
            samples = []

            def one_window():
                # Every X11 window the process owns, mapped or not.
                found = subprocess.run(["xdotool", "search", "--all", "--pid", str(proc.pid), "--name", "."],
                                       text=True, capture_output=True).stdout.split()
                samples.append(len(found))
                assert found == [window], f"window set changed: {found} (expected [{window}])"
                assert proc.poll() is None, "menu process exited"

            def until(predicate, seconds, message):
                def check():
                    one_window()
                    return predicate()
                return wait_until(check, seconds, message)

            def host_events():
                try:
                    return [e["event"] for e in json.loads(host.read_text())]
                except (FileNotFoundError, json.JSONDecodeError):
                    return []

            def grab(name):
                # The viewer's own swapchain capture (X root grabs are not
                # reliable under XWayland).
                (output / f"{name}.png").unlink(missing_ok=True)
                (output / f"{name}.request").touch()
                wait_until(lambda: (output / f"{name}.png").exists(), 10, f"no {name} capture")
                time.sleep(0.3)

            def last_page():
                pages = [e for e in host_events() if e.startswith("page")]
                return pages[-1] if pages else None

            xdo("windowfocus", "--sync", window)
            # Park the desktop pointer: hovering selects menu items.
            xdo("mousemove", "--window", window, 4, 4)
            # The book wakes, opens and turns to character select; Start is
            # the initial selection there.
            until(lambda: "page 9" in host_events(), 60, "character select did not open")
            time.sleep(0.5)
            # A key sent while the desktop moves focus is lost; only then
            # press again (a second press during the intro would skip it).
            for _ in range(3):
                xdo("windowfocus", "--sync", window)
                xdo("key", "--clearmodifiers", "Return")
                try:
                    until(lambda: "page 7" in host_events(), 3, "intro did not start")
                    break
                except AssertionError:
                    pass
            time.sleep(2)
            if skipped:
                xdo("key", "--clearmodifiers", "space")
            until(trace.exists, 120, "intro did not finish")
            events = json.loads(trace.read_text())
            loads = [e for e in events if "LoadFirstLevel" in e["event"]]
            assert len(loads) == 1, loads
            assert ("loading_tip: None" if skipped else "loading_tip: Some(7)") in loads[0]["event"], loads
            def sessions_seen():
                return [e for e in host_events() if not e.startswith("page")]
            until(lambda: sessions_seen()[:3] == ["launch", "begin", "end"], 120, "first session did not return")
            (output / "loading.json").rename(output / "session1-loading.json")
            (output / "playing.png").rename(output / "session1-playing.png")
            until(lambda: sessions_seen() == ["launch", "begin", "end", "launch", "begin", "end"], 120,
                  "second session did not return")
            assert all(e["windows"] == 1 for e in json.loads(host.read_text())), host.read_text()
            sessions = []
            for prefix in ["session1-", ""]:
                loading = json.loads((output / f"{prefix}loading.json").read_text())
                phases = [r for r in loading if "phase" in r]
                assert [r["phase"] for r in phases] == ["FadeIn", "Waiting", "FadeOut", "Complete"], phases
                complete = phases[-1]
                assert complete["assets_ready"] and complete["cue_finished"], complete
                assert complete["intro_tip"] == (not skipped), complete
                if not skipped:
                    assert complete["voice_seconds"], complete
                    assert complete["time"] >= max(complete["voice_seconds"]), complete
                sessions.append(phases)
            # The end-of-game page shows the capture while the camera pulls
            # back, then turns back to character select.
            time.sleep(0.4)
            grab("endgame")
            until(lambda: last_page() == "page 9", 10, "end of game did not return to character select")
            time.sleep(1.0)
            one_window()
            grab("returned")
            for name in ["loading", "session1-playing", "playing", "returned"]:
                image = Image.open(output / f"{name}.png").convert("RGB")
                assert max(ImageStat.Stat(image).stddev) > (0.5 if name == "loading" else 10), f"blank {name} screen"
            xdo("windowfocus", "--sync", window)
            xdo("mousemove", "--window", window, 4, 4)
            # Back one page at a time to the main page; there Escape attaches
            # the cursor, the next opens the quit box, and Return confirms Yes.
            def press(key, done, message, attempts=3, wait=3):
                # Keys sent while the desktop moves focus are lost: press
                # again only when the expected effect has not happened.
                for _ in range(attempts):
                    xdo("windowfocus", "--sync", window)
                    xdo("key", "--clearmodifiers", key)
                    try:
                        return wait_until(done, wait, message)
                    except AssertionError:
                        pass
                raise AssertionError(message)

            for page in ["page 1", "page 0"]:
                time.sleep(1.0)
                press("Escape", lambda: last_page() == page, f"Escape did not turn back to {page}")
            time.sleep(1.0)
            # The first Escape attaches the cursor, the next opens the box.
            press("Escape", lambda: "quit box" in host_events(), "the quit box did not open",
                  attempts=6, wait=0.8)
            time.sleep(0.5)
            grab("quit")
            press("Return", lambda: proc.poll() is not None, "Yes did not quit", wait=5)
            assert proc.wait(timeout=15) == 0
            result = {"skipped": skipped, "menu_pid": proc.pid, "window": window,
                      "window_samples": len(samples), "max_windows": max(samples),
                      "host_events": json.loads(host.read_text()), "sessions": sessions,
                      "captures": 6, "exit_code": proc.returncode}
            (output / "result.json").write_text(json.dumps(result, indent=2))
            print(json.dumps(result, indent=2), flush=True)
        finally:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGTERM)
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(proc.pid, signal.SIGKILL)
                    proc.wait()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--viewer", type=Path, default=Path("output/rewrite/target/release/magicka-viewer"))
    parser.add_argument("--content", type=Path, default=Path("target/Magicka/Content"))
    parser.add_argument("--cache", type=Path, default=Path("output/.cache"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--skipped", action="store_true")
    args = parser.parse_args()
    run(args.viewer, args.content, args.cache, args.output, args.skipped)
