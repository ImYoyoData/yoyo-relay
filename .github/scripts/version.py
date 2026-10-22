#!/usr/bin/env python3
"""Print the crate version from Cargo.toml (single source of truth)."""
import re
import sys

text = open("Cargo.toml", encoding="utf-8").read()
match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
if not match:
    sys.exit("Cargo.toml: package version not found")
print(match.group(1))
