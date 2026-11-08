#!/usr/bin/env python3
"""Reproduce the authored lab edit fixture without changing its source files."""
from pathlib import Path
import shutil

source = Path(__file__).resolve().parents[1] / 'scenarios'
destination = source.parents[1] / '.cache' / 'authored-scenarios' / 'moved'
destination.mkdir(parents=True, exist_ok=True)
for path in source.iterdir():
    if path.suffix in ('.lvl', '.xml', '.arena'):
        shutil.copyfile(path, destination / path.name)
arena = destination / 'lab.arena'
text = arena.read_text()
text = text.replace('0,2,-5', '8,2,-5').replace('-7,1,5', '-10,1,5')
text = text.replace('8,1,-7', '12,1,-7')
arena.write_text(text)
print(destination / 'feature-lab.lvl')
