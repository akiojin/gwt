#!/usr/bin/env python3
"""Issue #4135: map consolidated sources and compare compiled nextest inventories."""

import argparse
import json
from pathlib import Path
import re
import sys
import tomllib


def integration_sources(root):
    """Read Cargo's explicit targets and their source-module registrations."""
    crate = root / "crates/gwt"
    manifest = tomllib.loads((crate / "Cargo.toml").read_text(encoding="utf-8"))
    result = {}
    for target in manifest.get("test", []):
        source = crate / target["path"]
        modules = re.findall(r'#\[path\s*=\s*"([^"\n]+)"\]\s*mod\s+(\w+)\s*;', source.read_text(encoding="utf-8"))
        members = [(source, None)] + [(source.parent / path, name) for path, name in modules]
        for path, module in members:
            relative = path.resolve().relative_to(root.resolve()).as_posix()
            if relative in result:
                raise ValueError(f"duplicate integration source: {relative}")
            result[relative] = (target["name"], module)
    return result


def changed_targets(files, root):
    sources = integration_sources(root)
    return sorted({f"gwt|test|{sources[file][0]}" for file in files if file in sources})


def inventory_tests(inventory, root):
    sources = integration_sources(root)
    consolidated = {}
    for target, module in sources.values():
        if module is not None:
            consolidated.setdefault(target, set()).add(module)
    runtime_modules = set(re.findall(r"^mod (\w+_tests);", (root / "crates/gwt/src/app_runtime/tests.rs").read_text(encoding="utf-8"), re.MULTILINE))
    tests = {}
    for suite in inventory["rust-suites"].values():
        package, kind, binary = (suite[k] for k in ("package-name", "kind", "binary-name"))
        for original_name, test in suite["testcases"].items():
            name, target = original_name, binary
            if package == "gwt" and kind == "test" and binary in consolidated:
                module, sep, name = name.partition("::")
                if not sep or module not in consolidated[binary]:
                    raise ValueError(f"unregistered integration test: {binary}::{original_name}")
                target = module
            if package == "gwt" and kind == "bin" and binary == "gwt":
                prefix = "app_runtime::tests::"
                if name.startswith(prefix):
                    module, sep, rest = name[len(prefix):].partition("::")
                    if sep and module in runtime_modules:
                        name = prefix + rest
            key = "|".join((package, kind, target, name))
            if key in tests:
                raise ValueError(f"duplicate canonical test: {key}")
            tests[key] = test["ignored"]
    return tests


def compare(before, after):
    return {
        "before_count": len(before),
        "after_count": len(after),
        "missing": sorted(before.keys() - after.keys()),
        "ignored_changed": sorted(name for name in before.keys() & after.keys() if before[name] != after[name]),
        "added": sorted(after.keys() - before.keys()),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("targets", help="Read changed file paths from stdin")
    diff = commands.add_parser("compare", help="Reject missing tests or changed ignored flags")
    diff.add_argument("before", type=Path)
    diff.add_argument("after", type=Path)
    args = parser.parse_args()
    if args.command == "targets":
        print("\n".join(changed_targets(sys.stdin.read().splitlines(), args.root)))
        return 0
    inventories = [json.loads(path.read_text(encoding="utf-8-sig")) for path in (args.before, args.after)]
    result = compare(*(inventory_tests(inventory, args.root) for inventory in inventories))
    print(json.dumps(result, indent=2))
    return int(bool(result["missing"] or result["ignored_changed"]))


if __name__ == "__main__":
    raise SystemExit(main())
