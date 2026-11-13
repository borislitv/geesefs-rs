#!/usr/bin/env python3
"""Reject private dependency origins and a weakened package unsafe-code policy."""
import json
from pathlib import Path
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
PUBLIC_REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
# Explicitly authorized, maintained downstream fork. A different revision,
# repository, package or unpinned branch requires a new reviewed entry.
FOYER_REVISION = "d60dd0462c1de1e0853590791371208b659c90bf"
PUBLIC_GIT = {
    f"git+https://github.com/borislitv/foyer?rev={FOYER_REVISION}#{FOYER_REVISION}": {
        "foyer", "foyer-common", "foyer-memory", "foyer-storage", "foyer-tokio", "foyer-opendal",
    },
}


def violations(metadata, manifest, root=ROOT):
    errors = []
    if manifest.get("lints", {}).get("rust", {}).get("unsafe_code") != "forbid":
        errors.append('package must set lints.rust.unsafe_code = "forbid"')
    members = set(metadata["workspace_members"])
    for package in metadata["packages"]:
        name = package["name"]
        if package["id"] in members:
            if Path(package["manifest_path"]).resolve() != root / "Cargo.toml":
                errors.append(f"unexpected workspace source: {name}")
        elif (package.get("source") != PUBLIC_REGISTRY
              and name not in PUBLIC_GIT.get(package.get("source"), ())):
            errors.append(f"non-public dependency source: {name}")
        if not (package.get("license") or package.get("license_file")):
            errors.append(f"missing license declaration: {name}")
    return errors


def main():
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--all-features", "--format-version", "1"],
        cwd=ROOT, check=True, capture_output=True, text=True, timeout=120,
    )
    metadata = json.loads(result.stdout)
    errors = violations(metadata, manifest)
    print(json.dumps({"packages": len(metadata["packages"]), "errors": errors}, indent=2))
    return bool(errors)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"dependency audit failed: {error}", file=sys.stderr)
        sys.exit(1)
