#!/usr/bin/env python3
"""Writes THIRD_PARTY_LICENSES.md from Cargo.lock and cargo metadata.

Lists every crate the app can link on any desktop platform, with its version
and license expression. Build-only and test-only crates are left out.
Run from the repository root: python3 scripts/third-party-licenses.py
"""
import json
import subprocess
from collections import defaultdict

meta = json.loads(
    subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--locked"], text=True
    )
)
packages = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
ours = {p["id"] for p in meta["packages"] if p["source"] is None}

# Walk normal (non-build, non-dev) dependencies from our own crates.
seen = set()
stack = list(ours)
while stack:
    pid = stack.pop()
    for dep in nodes[pid]["deps"]:
        kinds = {k["kind"] for k in dep["dep_kinds"]}
        if kinds <= {"dev", "build"}:
            continue
        if dep["pkg"] not in seen:
            seen.add(dep["pkg"])
            stack.append(dep["pkg"])

by_license = defaultdict(list)
for pid in seen - ours:
    p = packages[pid]
    by_license[p.get("license") or "see the crate's own license file"].append(
        f"{p['name']} {p['version']}"
    )

lines = [
    "# Third Party Licenses",
    "",
    "Unweb is built with the Rust crates listed below, grouped by license. "
    "The list is generated from Cargo.lock by scripts/third-party-licenses.py and "
    "covers every crate the app can link on Windows, macOS or Linux; a given build "
    "links only the ones for its platform. Each crate's full license text ships "
    "with its source on crates.io.",
    "",
    "On each platform Unweb also uses the system's own web engine, which is "
    "not part of the download: WebView2 on Windows, WKWebView on macOS and "
    "WebKitGTK on Linux.",
    "",
]
for lic in sorted(by_license, key=lambda k: (-len(by_license[k]), k)):
    crates = sorted(by_license[lic])
    lines.append(f"## {lic}")
    lines.append("")
    lines.append(f"{len(crates)} crates: " + ", ".join(crates) + ".")
    lines.append("")

with open("THIRD_PARTY_LICENSES.md", "w") as f:
    f.write("\n".join(lines))
print(f"{len(seen - ours)} crates under {len(by_license)} license expressions")
