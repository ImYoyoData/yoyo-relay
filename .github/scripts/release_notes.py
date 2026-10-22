#!/usr/bin/env python3
"""Extract the release section for a version from CHANGELOG.md."""
import re
import sys

if len(sys.argv) != 2:
    sys.exit("usage: release_notes.py <version>")
version = sys.argv[1]

text = open("CHANGELOG.md", encoding="utf-8").read()
pattern = re.compile(
    r"(?ms)^##\s*\[?v?" + re.escape(version) + r"\]?[^\n]*\n(.*?)(?=^##\s|\Z)"
)
match = pattern.search(text)
if not match:
    sys.exit(f"CHANGELOG.md has no section for version {version}")

section = match.group(0).strip()
body = f"# YoyoRelay v{version}\n\n{section}\n"
open("release-notes.md", "w", encoding="utf-8").write(body)
print(body[:400])
