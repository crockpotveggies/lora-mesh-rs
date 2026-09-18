#!/usr/bin/env python3
"""Seed the local fuzz corpus from the independent public wire fixture (no private keys)."""
import json, pathlib
root = pathlib.Path(__file__).resolve().parents[1]
fixture = json.loads((root / 'tests/fixtures/secure-v2.json').read_text())
corpus = root / 'fuzz/corpus/parsers'; corpus.mkdir(parents=True, exist_ok=True)
for name in ('plaintext', 'secure_frame'):
    (corpus / name).write_bytes(bytes.fromhex(fixture[name]))
