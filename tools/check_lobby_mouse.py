#!/usr/bin/env python3
"""Exercise campaign lobby pointer choices, Start and saved-campaign resume on X11.
Run from the repository root under xvfb-run. Uses a private QA save directory.
"""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import time


def wait(predicate, seconds=60):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        value = predicate()
        if value:
            return value
        time.sleep(0.2)
    raise AssertionError('Timed out waiting for lobby action')


def read(path, fallback=None):
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return fallback


def run(args):
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    root = out / 'user'
    root.mkdir(exist_ok=True)
    for name in ['gltf', 'textures']:
        if not (root / name).exists():
            (root / name).symlink_to((args.cache / name).resolve(), target_is_directory=True)
    saves = root / 'saves'
    saves.mkdir(exist_ok=True)
    for slot in saves.glob('campaign-*.json'):
        slot.unlink()
    ids = ['00000000000000000000000000000001', '00000000000000000000000000000002']
    settings = {'version': 1, 'music': 0.0, 'sfx': 0.0, 'history': [], 'profiles': [
        {'id': id_, 'name': name, 'robe': 'Wizard', 'colour': [1.0, 1.0, 1.0]}
        for id_, name in zip(ids, ['First', 'Second'])]}
    (saves / 'settings.json').write_text(json.dumps(settings))
    results = []
    for case in ['new-mouse', 'continue-keyboard', 'continue-mouse']:
        folder = out / case
        folder.mkdir(exist_ok=True)
        (folder / 'host.json').unlink(missing_ok=True)
        env = os.environ.copy()
        env.pop('WAYLAND_DISPLAY', None)
        env.pop('WAYLAND_SOCKET', None)
        command = [str(args.viewer.resolve()), '--menu', str(args.content.resolve()), '--root', str(root),
                   '--menu-page', '0', '--silent', '--host-audit', str(folder), '--screenshots', str(folder)]
        with (folder / 'viewer.log').open('w') as log:
            proc = subprocess.Popen(command, env=env, stdout=log, stderr=log, start_new_session=True)
            try:
                def xdo(*words):
                    return subprocess.check_output(['xdotool', *map(str, words)], text=True, timeout=60).strip()
                window = xdo('search', '--sync', '--pid', proc.pid, '--name', '^Magicka rewrite$').splitlines()[0]
                xdo('windowfocus', '--sync', window)
                def events():
                    assert proc.poll() is None, f'viewer exited: {proc.returncode}'
                    return [event['event'] for event in read(folder / 'host.json', [])]
                def grab(name):
                    (folder / f'{name}.png').unlink(missing_ok=True)
                    (folder / f'{name}.request').touch()
                    wait(lambda: (folder / f'{name}.png').exists(), 15)
                def click(x, y):
                    xdo('mousemove', '--window', window, '--sync', x, y)
                    time.sleep(0.3)
                    xdo('click', 1)
                    time.sleep(0.5)
                wait(lambda: 'page 0' in events())
                time.sleep(2)
                grab('main')
                click(350, 270)
                wait(lambda: 'page 1' in events())
                time.sleep(1)
                grab('slots')
                click(500, 150)
                wait(lambda: 'page 9' in events())
                time.sleep(2.5)
                grab('lobby')
                if case == 'new-mouse':
                    click(580, 144)
                    grab('profiles')
                    click(600, 128)
                    grab('robes')
                    click(543, 133)
                    grab('colours')
                    click(683, 123)
                    grab('chosen')
                    current = read(saves / 'settings.json')
                    assert current['profiles'][1]['robe'] != 'Wizard', current
                    expected = [363 / 255, 298 / 255, 0.0]
                    assert all(abs(a - b) < 1e-6 for a, b in
                               zip(current['profiles'][1]['colour'], expected)), current
                if case == 'continue-mouse':
                    # Resume directly from a robe editor: Start must retain
                    # its own action while a customization row is open.
                    click(580, 144)
                    click(600, 128)
                    xdo('mousemove', '--window', window, '--sync', 600, 133)
                    time.sleep(0.5)
                    grab('editing-before-start')
                if case == 'continue-keyboard':
                    xdo('mousemove', '--window', window, '--sync', 650, 630)
                    time.sleep(0.3)
                    xdo('windowfocus', '--sync', window)
                    time.sleep(0.5)
                    assert xdo('getwindowfocus') == window
                    xdo('key', 'Return')
                else:
                    click(650, 630)
                grab('activated')
                wait(lambda: 'page 7' in events() or 'launch' in events(), 15)
                if 'page 7' in events():
                    time.sleep(2)
                    xdo('key', 'space')
                wait(lambda: 'begin' in events(), 120)
                time.sleep(3)
                grab('playing')
                saved = read(saves / 'campaign-0.json')
                assert saved['players'][0]['id'] == ids[1], saved['players']
                result = {'case': case, 'events': events(), 'saved_level': saved['level'],
                          'profile': read(saves / 'settings.json')['profiles'][1]}
                results.append(result)
                print(json.dumps(result), flush=True)
            finally:
                if proc.poll() is None:
                    os.killpg(proc.pid, signal.SIGTERM)
                    proc.wait(timeout=15)
    (out / 'result.json').write_text(json.dumps(results, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--viewer', type=Path, default=Path('output/rewrite/target/release/magicka-viewer'))
    parser.add_argument('--content', type=Path, default=Path('target/Magicka/Content'))
    parser.add_argument('--cache', type=Path, default=Path('output/.cache'))
    parser.add_argument('--output', type=Path, required=True)
    run(parser.parse_args())
