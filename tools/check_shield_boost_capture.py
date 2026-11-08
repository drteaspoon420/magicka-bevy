#!/usr/bin/env python3
"""Real-input shield boost, held-key suppression and native animation audit."""
import json
import math
import re
import sys
from pathlib import Path

run = Path(sys.argv[1])
rows = {r['capture']: r for r in json.loads((run / 'log.json').read_text())}
pattern = r'autopilot (\w+): shield boost state (\w+) count (-?\d+) cooldown (\S+) hp (\[[^\n]*\])'
states = {name: (state, int(count), float(cooldown), json.loads(hp))
          for name, state, count, cooldown, hp in re.findall(pattern, (run / 'viewer.log').read_text())}
names = ('before', 'boost', 'held', 'released', 'repressed')
assert all(len(states[n][3]) == 1 for n in names), 'Expected one live shield'
health = {n: states[n][3][0] for n in names}
def gained(start, end):
    # Native healing 100 HP, decay 10 HP every 0.1 s; at most one boundary
    # tick beyond ceil(duration/period). State-entry consumes one update.
    decay_bound = (math.ceil((rows[end]['frame'] - rows[start]['frame']) / 60 / .1) + 1) * 10
    return 100 - decay_bound <= health[end] - health[start] <= 100
passed = (gained('before', 'boost') and gained('released', 'repressed')
          and health['held'] < health['boost']
          and rows['boost']['animation'] == 'boost'
          and rows['repressed']['animation'] == 'boost'
          and states['boost'][0] == states['repressed'][0] == 'Boost'
          and states['held'][0] == 'Idle')
report = {'passed': passed, 'health': health,
          'first_boost_net_healing': health['boost'] - health['before'],
          'held_key_health_change': health['held'] - health['boost'],
          'repress_net_healing': health['repressed'] - health['released'],
          'native_visual_parity': 'unverified'}
(run / 'shield-boost-check.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
if not passed:
    raise SystemExit('Shield boost input/state regression')
