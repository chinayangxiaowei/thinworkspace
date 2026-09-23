#!/usr/bin/env python3
"""Reject product-crate dependency edges outside the approved Phase 1 graph."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


ALLOWED_LOCAL_DEPENDENCIES: dict[str, frozenset[str]] = {
    "thinws-core": frozenset(),
    "thinws-ports": frozenset({"thinws-core"}),
    "thinws-application": frozenset({"thinws-core", "thinws-ports"}),
    "thinws-cli": frozenset({"thinws-application"}),
    "thinws-adapter-git-cli": frozenset({"thinws-core", "thinws-ports"}),
    "thinws-adapter-macos": frozenset({"thinws-core", "thinws-ports"}),
    "thinws-metadata-sqlite": frozenset({"thinws-core", "thinws-ports"}),
}

# P1-01 freezes Core's complete direct dependency surface. Other product
# crates add their external allowlist when their own implementation task begins.
STRICT_EXTERNAL_DEPENDENCIES: dict[str, frozenset[str]] = {
    "thinws-core": frozenset({"thiserror", "uuid"}),
}


def validate_dependency_graph(graph: dict[str, set[str]]) -> list[str]:
    """Return stable diagnostics for every currently enforceable dependency edge."""
    violations: list[str] = []
    for package in sorted(graph):
        allowed_local = ALLOWED_LOCAL_DEPENDENCIES.get(package)
        if allowed_local is None:
            violations.append(f"{package} has no approved dependency rule")
            continue
        allowed_external = STRICT_EXTERNAL_DEPENDENCIES.get(package)
        for dependency in sorted(graph[package]):
            if dependency in allowed_local:
                continue
            if allowed_external is not None and dependency in allowed_external:
                continue
            if allowed_external is not None or dependency.startswith("thinws-"):
                violations.append(f"{package} must not depend on {dependency}")
    return violations


def load_product_graph(repository: Path) -> dict[str, set[str]]:
    """Read the current workspace graph from Cargo without resolving new versions."""
    completed = subprocess.run(
        [
            "cargo",
            "metadata",
            "--locked",
            "--format-version",
            "1",
            "--no-deps",
        ],
        cwd=repository,
        check=True,
        capture_output=True,
        text=True,
    )
    metadata = json.loads(completed.stdout)
    members = set(metadata["workspace_members"])
    packages = {
        package["id"]: package
        for package in metadata["packages"]
        if package["id"] in members
    }
    product_packages = {
        package["name"]: package
        for package in packages.values()
        if _is_product_path(repository, package["manifest_path"])
    }
    return {
        package["name"]: {dependency["name"] for dependency in package["dependencies"]}
        for package in product_packages.values()
    }


def _is_product_path(repository: Path, path: str) -> bool:
    product_root = (repository / "crates").resolve()
    try:
        Path(path).resolve().relative_to(product_root)
    except ValueError:
        return False
    return True


def main() -> int:
    repository = Path(__file__).resolve().parents[1]
    try:
        graph = load_product_graph(repository)
    except (OSError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        print(f"failed to inspect Cargo dependency graph: {error}", file=sys.stderr)
        return 2

    violations = validate_dependency_graph(graph)
    if violations:
        print("crate dependency direction violations:", file=sys.stderr)
        for violation in violations:
            print(f"- {violation}", file=sys.stderr)
        return 1

    print(f"crate dependency direction valid for {len(graph)} product crate(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
