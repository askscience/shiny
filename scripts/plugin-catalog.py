#!/usr/bin/env python3
"""The published plugin catalogue (index.json), kept across publications.

A plugin is published when its version in plugins/<name>/plugin.toml goes up
on main (.github/workflows/publish-plugins.yml). The catalogue therefore
grows one plugin at a time instead of being rewritten by a release:

  plugin-catalog.py pending <published-index.json> [--arch amd64]
      Names of the process plugins whose version in the repository is newer
      than anything the catalogue has for them, one per line. A missing or
      empty catalogue file means nothing is published yet.

  plugin-catalog.py merge <published-index.json> <built-index.json> <out.json>
      The published catalogue plus what was just built.

A catalogue may hold several versions of one plugin: a version can need a
newer server (`min_host`), and servers that were not updated keep the last
version made for them. `merge` keeps, per plugin and architecture, each
version that is the newest one some server can run and drops the rest.
"""
import json
import pathlib
import re
import sys
import tomllib

REPO = pathlib.Path(__file__).resolve().parent.parent
FORMAT = 2


def version_key(text):
    """Order versions as the server does (semver, numeric components)."""
    match = re.fullmatch(r"\s*(\d+)\.(\d+)\.(\d+)\s*", str(text or ""))
    if not match:
        raise ValueError(f"not a version: {text!r}")
    return tuple(int(part) for part in match.groups())


def min_host_key(entry):
    """The oldest server an entry runs on; none stated means any."""
    text = str(entry.get("min_host") or "").strip()
    return version_key(text) if text else (0, 0, 0)


def load_index(path):
    file = pathlib.Path(path)
    if not file.is_file() or not file.read_text().strip():
        return {"format": FORMAT, "release": "", "plugins": []}
    index = json.loads(file.read_text())
    index.setdefault("plugins", [])
    return index


def repo_plugins(repo=REPO):
    """name -> manifest of every process plugin the catalogue publishes."""
    found = {}
    for manifest in sorted(repo.glob("plugins/*/plugin.toml")):
        name = manifest.parent.name
        if name == "hello":
            continue
        with manifest.open("rb") as handle:
            data = tomllib.load(handle)
        if data.get("runtime") != "process":
            continue
        found[name] = data
    return found


def pending(index, plugins, arch):
    """Plugins whose repository version the catalogue does not have yet."""
    names = []
    for name, manifest in plugins.items():
        version = version_key(manifest["version"])
        min_host_key(manifest)  # a malformed min_host fails here, not on users
        published = [
            version_key(entry["version"])
            for entry in index["plugins"]
            if entry.get("name") == name and entry.get("arch") == arch
        ]
        if not published or version > max(published):
            names.append(name)
        elif version < max(published):
            print(
                f"warning: {name} {manifest['version']} is older than the published version; not published",
                file=sys.stderr,
            )
    return names


def merge(published, built):
    """`published` with the entries of `built` added, superseded ones dropped."""
    entries = {}
    for entry in list(published["plugins"]) + list(built["plugins"]):
        # A rebuilt version replaces the published one of the same number.
        entries[(entry["name"], entry["arch"], version_key(entry["version"]))] = entry

    kept = []
    groups = {}
    for (name, arch, _version), entry in entries.items():
        groups.setdefault((name, arch), []).append(entry)
    for group in groups.values():
        group.sort(key=lambda e: version_key(e["version"]), reverse=True)
        lowest = None
        for entry in group:
            need = min_host_key(entry)
            # Kept only when it reaches servers no newer version reaches.
            if lowest is None or need < lowest:
                kept.append(entry)
                lowest = need
    kept.sort(key=lambda e: (e["name"], e["arch"], version_key(e["version"])))
    return {
        "format": FORMAT,
        "release": built.get("release") or published.get("release", ""),
        "plugins": kept,
    }


def main(argv):
    if len(argv) >= 2 and argv[0] == "pending":
        arch = "amd64"
        if "--arch" in argv:
            arch = argv[argv.index("--arch") + 1]
        for name in pending(load_index(argv[1]), repo_plugins(), arch):
            print(name)
        return 0
    if len(argv) == 4 and argv[0] == "merge":
        merged = merge(load_index(argv[1]), load_index(argv[2]))
        pathlib.Path(argv[3]).write_text(json.dumps(merged, indent=2) + "\n")
        print(f"catalogue: {argv[3]} ({len(merged['plugins'])} entries)", file=sys.stderr)
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
