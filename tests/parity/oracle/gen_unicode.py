"""Dump the Unicode facts Accounts depends on, from the oracle's Python: `str.lower()` for every
code point it changes, and the `str.strip()` whitespace set. Run with the oracle's interpreter:
    .venv/bin/python -B gen_unicode.py > tests/fixtures/python_unicode.json
"""

import json
import sys
import unicodedata

lower = {}
space = []
for cp in range(0x110000):
    if 0xD800 <= cp <= 0xDFFF:
        continue
    c = chr(cp)
    low = c.lower()
    if low != c:
        lower[f"{cp:X}"] = [ord(x) for x in low]
    if c.strip() == "" and c != "":
        space.append(cp)
json.dump(
    {
        "python": sys.version.split()[0],
        "unicode": unicodedata.unidata_version,
        "lower": lower,
        "strip_whitespace": space,
    },
    sys.stdout,
    separators=(",", ":"),
    sort_keys=True,
)
