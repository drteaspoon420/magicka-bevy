#!/usr/bin/env python3
"""Decode Grimnir's FieldRVA initializer without loading/executing target code."""
import hashlib
import json
from pathlib import Path
import struct
import sys

sys.path.insert(0, "output/evidence/tools")
from pe_static import PEImage

target = Path("target/Magicka/Magicka.exe")
data = target.read_bytes()
rva = 0x19E6B8
offset = PEImage(data).get_offset_from_rva(rva)
record = {
    "target": str(target),
    "sha256": hashlib.sha256(data).hexdigest(),
    "field_token": "0x04004665",
    "rva": hex(rva),
    "file_offset": offset,
    "bytes": data[offset:offset + 24].hex(),
    "spawn_counts": struct.unpack_from("<6i", data, offset),
}
output = Path("output/evidence/boss-grimnir/spawn-counts.json")
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(
    json.dumps(record, indent=2) + "\n"
)
print(json.dumps(record))
