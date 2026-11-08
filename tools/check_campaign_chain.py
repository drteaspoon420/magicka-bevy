#!/usr/bin/env python3
"""Drive the campaign chain in one process and one X11 window, using real X11
input: character select at chapter 2 -> SubMenuCutscene (chapter page, map) ->
loading -> Mountaindale -> LevelComplete -> SubMenuEndGame -> chapter 3 map ->
loading -> Highlands, and check the save slot moved on to Highlands."""
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
        time.sleep(0.1)
    raise AssertionError(message)


def run(viewer, content, cache, output):
    output.mkdir(parents=True, exist_ok=True)
    for path in output.glob("*.png"):
        path.unlink()
    (output / "host.json").unlink(missing_ok=True)
    user = output / "user"
    user.mkdir(exist_ok=True)
    for name in ["gltf", "textures"]:
        asset_dir = user / name
        if not asset_dir.exists():
            asset_dir.symlink_to((cache / name).resolve(), target_is_directory=True)
    saves = user / "saves"
    for slot in saves.glob("campaign-*.json"):
        slot.unlink()
    env = os.environ.copy()
    env.pop("WAYLAND_DISPLAY", None)
    env.pop("WAYLAND_SOCKET", None)
    command = [str(viewer.resolve()), "--menu", str(content.resolve()), "--root", str(user.resolve()),
               "--menu-page", "9", "--menu-campaign", "Mountaindale.lvl", "--silent",
               "--audit-complete-after", "4", "--host-audit", str(output.resolve()),
               "--screenshots", str(output.resolve())]
    host = output / "host.json"
    with (output / "viewer.log").open("w") as log:
        proc = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            window = xdo("search", "--sync", "--all", "--pid", proc.pid, "--name", "^Magicka rewrite$").splitlines()[0]

            def events():
                try:
                    return [e for e in json.loads(host.read_text())]
                except (FileNotFoundError, json.JSONDecodeError):
                    return []

            def names():
                return [e["event"] for e in events()]

            def until(predicate, seconds, message):
                def check():
                    assert proc.poll() is None, "menu process exited"
                    return predicate()
                return wait_until(check, seconds, message)

            def grab(name):
                # The viewer's own swapchain capture.
                (output / f"{name}.png").unlink(missing_ok=True)
                (output / f"{name}.request").touch()
                wait_until(lambda: (output / f"{name}.png").exists(), 10, f"no {name} capture")
                time.sleep(0.2)

            xdo("windowfocus", "--sync", window)
            # Park the pointer: hovering selects menu items.
            xdo("mousemove", "--window", window, 4, 4)
            until(lambda: "page 9" in names(), 60, "character select did not open")
            time.sleep(0.5)
            # A key sent while the desktop moves focus is lost; press again.
            for _ in range(3):
                xdo("windowfocus", "--sync", window)
                xdo("key", "--clearmodifiers", "Return")
                try:
                    until(lambda: "page 12" in names(), 3, "the chapter map did not open")
                    break
                except AssertionError:
                    pass
            # Chapter 2: the map cutscene, then the level.
            until(lambda: "page 12" in names(), 20, "the chapter map did not open")
            time.sleep(4.5)
            grab("map1")
            until(lambda: names().count("begin") >= 1, 60, "chapter 2 did not begin")
            # LevelComplete -> SubMenuEndGame -> chapter 3's map.
            until(lambda: names().count("end") >= 1, 60, "chapter 2 did not complete")
            time.sleep(0.4)
            grab("endgame")
            until(lambda: names().count("page 12") >= 2, 20, "chapter 3's map did not open")
            time.sleep(4.5)
            grab("map2")
            until(lambda: names().count("begin") >= 2, 60, "chapter 3 did not begin")
            time.sleep(1.0)
            grab("chapter3")
            sequence = [n for n in names() if n in ("launch", "begin", "end", "page 11", "page 12")]
            assert sequence[:8] == ["page 12", "launch", "begin", "end", "page 11", "page 12", "launch", "begin"], sequence
            assert all(e["windows"] == 1 for e in events()), events()
            slot = json.loads((saves / "campaign-0.json").read_text())
            assert "highlands" in json.dumps(slot.get("level", "")).lower(), slot.get("level")
            for name in ["map1", "endgame", "map2", "chapter3"]:
                image = Image.open(output / f"{name}.png").convert("RGB")
                assert max(ImageStat.Stat(image).stddev) > 10, f"blank {name} screen"
            result = {"menu_pid": proc.pid, "window": window, "host_events": events(),
                      "saved_level": slot.get("level"), "captures": 4}
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
    args = parser.parse_args()
    run(args.viewer, args.content, args.cache, args.output)
