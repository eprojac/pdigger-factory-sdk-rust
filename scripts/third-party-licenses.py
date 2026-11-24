#!/usr/bin/env python3
"""Writes the licence and notice texts of every crate the pdigger-factory-viewer binary is built from.

The release archives of the viewer carry the output beside the binary, as THIRD-PARTY-LICENSES.txt: the binary links
those crates (and embeds egui's default fonts), and their licences (MIT, BSD, ISC, OpenSSL-style, the font licences)
ask for their notices with every binary copy.

    python3 scripts/third-party-licenses.py [--target <triple>] > THIRD-PARTY-LICENSES.txt

It reads `cargo metadata --locked` (from the repository root, after a build, so that the crates' sources are there):
the normal dependencies of pdigger-factory-viz on that target, followed from the crate down, and for each crate its
licence expression and the licence, notice and copyright files it ships. Standard library only.
"""

import argparse
import json
import subprocess
import sys
import textwrap
from pathlib import Path

ROOT_CRATE = "pdigger-factory-viz"
# File names (lower case) that hold a licence, a notice or a copyright, at a crate's top level or in LICENSES/.
PREFIXES = ("license", "licence", "copying", "notice", "copyright", "unlicense", "authors")
# Crates that keep their licence texts elsewhere: egui's default fonts, whose licences are in fonts/.
EXTRA_DIRS = {"epaint_default_fonts": ("fonts",)}
CODE = {".rs", ".toml", ".py", ".sh", ".yml", ".yaml", ".json", ".lock", ".c", ".h", ".ttf", ".otf"}


def licence_files(pkg: dict) -> list[Path]:
    """The licence, notice and copyright files of one crate, in a stable order."""
    top = Path(pkg["manifest_path"]).parent
    found: set[Path] = set()
    for d in (top, top / "LICENSES"):
        if d.is_dir():
            found.update(
                f for f in d.iterdir() if f.is_file() and f.name.lower().startswith(PREFIXES) and f.suffix.lower() not in CODE
            )
    for x in EXTRA_DIRS.get(pkg["name"], ()):
        found.update(f for f in (top / x).glob("*.txt") if f.is_file())
    if pkg.get("license_file") and (top / pkg["license_file"]).is_file():
        found.add(top / pkg["license_file"])
    return sorted(found, key=lambda f: str(f.relative_to(top)).lower())


def closure(meta: dict) -> list[dict]:
    """The packages the root crate needs at run time on the chosen target (normal dependencies, transitively)."""
    by_id = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = next(p["id"] for p in meta["packages"] if p["name"] == ROOT_CRATE and p["source"] is None)
    seen, todo = {root}, [root]
    while todo:
        for dep in nodes[todo.pop()]["deps"]:
            if any(k["kind"] is None for k in dep["dep_kinds"]) and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                todo.append(dep["pkg"])
    # The workspace's own crates are this repository's (LICENSE and NOTICE ship beside this file).
    return sorted((by_id[i] for i in seen if by_id[i]["source"] is not None), key=lambda p: (p["name"], p["version"]))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--target", help="the target triple the viewer is built for (default: the host)")
    ap.add_argument("--offline", action="store_true", help="pass --offline to cargo")
    args = ap.parse_args()
    cmd = ["cargo", "metadata", "--format-version", "1", "--locked"]
    if args.target:
        cmd += ["--filter-platform", args.target]
    if args.offline:
        cmd.append("--offline")
    meta = json.loads(subprocess.run(cmd, check=True, capture_output=True, text=True).stdout)
    crates = closure(meta)
    viz = next(p for p in meta["packages"] if p["name"] == ROOT_CRATE and p["source"] is None)
    # Each text once, with the crates that ship it (most crates ship the same MIT and Apache-2.0 texts).
    texts: dict[str, list[str]] = {}
    without = []
    for p in crates:
        files = licence_files(p)
        if not files:
            without.append(f"{p['name']} {p['version']}")
        top = Path(p["manifest_path"]).parent
        for f in files:
            text = f.read_text(encoding="utf-8", errors="replace").strip()
            texts.setdefault(text, []).append(f"{p['name']} {p['version']} ({f.relative_to(top).as_posix()})")
    out = sys.stdout
    target = f" ({args.target})" if args.target else ""
    out.write(f"Third-party licences of pdigger-factory-viewer {viz['version']}{target}\n\n")
    out.write(
        f"The viewer binary is built from these {len(crates)} crates (Cargo.lock), each under the licence it states\n"
        "(Cargo.toml). The texts of their licence, notice and copyright files follow the list, each text once, with the\n"
        "crates that ship it.\n\n"
    )
    for p in crates:
        out.write(f"  {p['name']} {p['version']}: {p.get('license') or '(its licence file)'}\n")
    if without:
        note = (
            "These crates ship no licence file of their own; each is under the licence it states above (the standard "
            "texts of those licences are at https://spdx.org/licenses/): " + ", ".join(without) + "."
        )
        out.write("\n" + textwrap.fill(note, 116) + "\n")
    for text, users in texts.items():
        out.write("\n" + "=" * 100 + "\n")
        out.write("Shipped by " + ", ".join(users) + ":\n\n" + text + "\n")
    print(f"{len(crates)} crates, {len(texts)} distinct texts, {len(without)} crates without a licence file", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
