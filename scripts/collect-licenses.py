#!/usr/bin/env python3
"""Collect actual license texts for the locked platform dependency graph.

No network fallback during builds: missing package texts fail packaging.
Checked-in supplements distinguish upstream texts, canonical SPDX texts,
and exact registry source material; provenance is retained for each file.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys


def license_name(path):
    return path.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE"))


def collect(target, output):
    project = Path(__file__).resolve().parent.parent
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--filter-platform", target,
         "--format-version", "1"], cwd=project, check=True, capture_output=True, text=True,
    )
    metadata = json.loads(result.stdout)
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    reachable, pending = set(), [metadata["resolve"]["root"]]
    while pending:
        current = pending.pop()
        if current in reachable:
            continue
        reachable.add(current)
        for dependency in nodes[current]["deps"]:
            if any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])

    # Output is intentionally rebuilt so old versions cannot mask missing texts.
    if output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True)
    packages, missing = [], []
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["id"] not in reachable or package["source"] is None:
            continue
        key = f"{package['name']}-{package['version']}"
        source = Path(package["manifest_path"]).parent
        destination = output / key
        texts = [path for path in source.rglob("*") if path.is_file() and license_name(path)]
        license_file = package.get("license_file")
        if license_file:
            declared = Path(license_file)
            if not declared.is_absolute():
                declared = source / declared
            if declared.is_file() and declared not in texts:
                texts.append(declared)
        files = []
        for path in sorted(texts):
            try:
                relative = path.relative_to(source)
            except ValueError:
                relative = Path(path.name)
            if ".." in relative.parts:
                raise ValueError(f"Unsafe license path for {key}")
            copied = destination / relative
            copied.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, copied)
            files.append({"file": str(relative), "origin": "crate source"})

        supplement = project / "resources" / "dependency-licenses" / key
        if supplement.is_dir():
            provenance = json.loads((supplement / "provenance.json").read_text())
            if (provenance["name"], provenance["version"]) != (package["name"], package["version"]):
                raise ValueError(f"Supplement identity mismatch for {key}")
            for notice in provenance["files"]:
                relative = Path(notice["file"])
                if relative.is_absolute() or ".." in relative.parts:
                    raise ValueError(f"Unsafe supplement path for {key}")
                text = supplement / relative
                if hashlib.sha256(text.read_bytes()).hexdigest() != notice["sha256"]:
                    raise ValueError(f"Supplement checksum mismatch: {key}/{relative}")
                destination.mkdir(parents=True, exist_ok=True)
                copied = destination / relative
                copied.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(text, copied)
                files.append({"file": str(relative), "origin": notice["url"],
                              "kind": notice.get("kind", "upstream license or notice text")})
            shutil.copyfile(supplement / "provenance.json", destination / "provenance.json")
        if not files:
            missing.append(f"{key} ({package.get('license') or 'no declared license'})")
        packages.append({"name": package["name"], "version": package["version"],
                         "license": package.get("license"),
                         "repository": package.get("repository"),
                         "authors": package.get("authors", []), "files": files})
    (output / "index.json").write_text(json.dumps(
        {"target": target, "packages": packages, "missing_license_texts": missing}, indent=2
    ) + "\n")
    if missing:
        print("Missing actual license texts; packaging stopped:\n" + "\n".join(missing), file=sys.stderr)
        return 1
    print(f"Collected license texts for {len(packages)} dependencies into {output}")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=("aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu"))
    arguments = parser.parse_args()
    output = Path(__file__).resolve().parent.parent / "dist" / "license-notices"
    try:
        return collect(arguments.target, output)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f"License collection failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
