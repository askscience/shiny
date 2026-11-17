#!/usr/bin/env python3
"""Turn a library plugin into a process plugin.

Adds the `[[bin]]` target and its one-line `src/main.rs`, drops the `cdylib`
crate type (the library stays an rlib the binary links), marks
`runtime = "process"` in plugin.toml and bumps the plugin's patch version.

    scripts/plugin-to-process.py <name> <crate::path::ToPlugin> [--idle] [constructor]

--idle lets the server stop the process after ten idle minutes (only for
plugins without background work started in on_load).
"""
import re
import sys
from pathlib import Path

args = [a for a in sys.argv[1:] if a != "--idle"]
idle = "--idle" in sys.argv
name, path = args[0], args[1]
ctor = args[2] if len(args) > 2 else f"{path}::new()"
root = Path(__file__).resolve().parent.parent / "plugins" / name


def bump(version: str) -> str:
    major, minor, patch = (int(x) for x in version.split("."))
    # After x.y.9 comes x.(y+1).0.
    return f"{major}.{minor + 1}.0" if patch >= 9 else f"{major}.{minor}.{patch + 1}"


cargo = (root / "Cargo.toml").read_text()
if "[[bin]]" not in cargo:
    cargo = cargo.replace('crate-type = ["cdylib", "rlib"]', 'crate-type = ["rlib"]')
    cargo = re.sub(r"(\n\[dependencies\])", f'\n[[bin]]\nname = "shiny-plugin-{name}"\npath = "src/main.rs"\n\\1', cargo, count=1)
    (root / "Cargo.toml").write_text(cargo)

main = root / "src" / "main.rs"
if idle:
    body = (
        f"    shiny_plugin_sdk::process::run_with(\n"
        f"        {ctor},\n"
        f"        shiny_plugin_sdk::process::Options {{\n"
        f"            // No background work: the server may stop us when idle.\n"
        f"            idle_stop: Some(std::time::Duration::from_secs(600)),\n"
        f"        }},\n"
        f"    );\n"
    )
else:
    body = f"    shiny_plugin_sdk::process::run({ctor});\n"
main.write_text(
    f"//! The {name} plugin as its own process (`runtime = \"process\"`).\n\n"
    f"fn main() {{\n{body}}}\n"
)

toml = (root / "plugin.toml").read_text()
if "runtime" not in toml:
    toml = re.sub(r'^version = "([^"]+)"', lambda m: f'version = "{bump(m.group(1))}"', toml, count=1, flags=re.M)
    toml = re.sub(r"^(api_level = .*)$", r'\1\nruntime = "process"', toml, count=1, flags=re.M)
    (root / "plugin.toml").write_text(toml)
print(f"{name}: process plugin")
