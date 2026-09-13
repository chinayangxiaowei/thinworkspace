#!/usr/bin/env python3
"""Run explicitly scoped mutation testing for the P0 workflow."""

from __future__ import annotations

import argparse
import json
import stat
import subprocess
import sys
from pathlib import Path
from typing import Mapping


REPO_ROOT = Path(__file__).resolve().parent.parent
SCOPES = ("modules", "crates", "workspace")
GLOB_CHARACTERS = frozenset("*?[]{}!")
TEST_COMPONENTS = frozenset({"test", "tests", "bench", "benches", "example", "examples"})
FIXED_ARGUMENTS = [
    "--leak-dirs",
    "--timeout",
    "60",
    "--jobs",
    "2",
    "--output",
    "target",
    "--test-workspace",
    "true",
    "--",
    "--",
    "--include-ignored",
    "--nocapture",
]


class SelectionError(ValueError):
    """The requested mutation scope is not an exact workspace selection."""


def _targets(raw_targets: str, *, required: bool) -> list[str]:
    if not raw_targets.strip():
        if required:
            raise SelectionError("this mutation scope requires at least one target")
        return []

    targets = [target.strip() for target in raw_targets.split(",")]
    if any(not target or "\0" in target for target in targets):
        raise SelectionError("mutation targets must be non-empty comma-separated values")
    if len(targets) != len(set(targets)):
        raise SelectionError("mutation targets must not be repeated")
    return targets


def _is_dedicated_test_source(relative_source: Path) -> bool:
    if any(part.lower() in TEST_COMPONENTS for part in relative_source.parts[1:-1]):
        return True
    stem = relative_source.stem.lower()
    return (
        stem in {"test", "tests"}
        or stem.startswith("test_")
        or stem.endswith("_test")
        or stem.endswith("_tests")
    )


def _validate_module(
    repo_root: Path,
    target: str,
    package_roots: Mapping[str, Path],
) -> str:
    if "\\" in target or any(character in GLOB_CHARACTERS for character in target):
        raise SelectionError("module targets must be exact repository-relative paths")

    relative = Path(target)
    if (
        relative.is_absolute()
        or relative.as_posix() != target
        or any(part in {"", ".", ".."} for part in relative.parts)
        or relative.suffix != ".rs"
    ):
        raise SelectionError("module targets must be normalized relative Rust paths")

    candidate = repo_root.joinpath(relative)
    try:
        identity = candidate.lstat()
    except OSError as error:
        raise SelectionError("module target must name an existing source file") from error
    if not stat.S_ISREG(identity.st_mode):
        raise SelectionError("module target must be a regular source file")

    try:
        resolved_candidate = candidate.resolve(strict=True)
        resolved_candidate.relative_to(repo_root)
    except (OSError, ValueError) as error:
        raise SelectionError("module target must remain inside the repository") from error

    for package_root in package_roots.values():
        try:
            package_source = package_root.resolve(strict=True) / "src"
            relative_source = resolved_candidate.relative_to(package_source)
        except (OSError, ValueError):
            continue
        if not relative_source.parts or _is_dedicated_test_source(
            Path("src") / relative_source
        ):
            raise SelectionError("module target must be production source")
        return target

    raise SelectionError("module target must be under a workspace package src directory")


def build_mutation_command(
    repo_root: Path,
    scope: str,
    raw_targets: str,
    package_roots: Mapping[str, Path],
) -> list[str]:
    """Build the fixed cargo-mutants argv for one validated selection."""
    try:
        repo_root = repo_root.resolve(strict=True)
    except OSError as error:
        raise SelectionError("repository root must exist") from error

    command = ["cargo", "mutants"]
    if scope == "modules":
        targets = _targets(raw_targets, required=True)
        command.append("--workspace")
        for target in targets:
            command.extend(
                ["--file", _validate_module(repo_root, target, package_roots)]
            )
    elif scope == "crates":
        targets = _targets(raw_targets, required=True)
        for target in targets:
            if target not in package_roots:
                raise SelectionError(f"unknown workspace package: {target}")
            command.extend(["--package", target])
    elif scope == "workspace":
        if _targets(raw_targets, required=False):
            raise SelectionError("workspace scope does not accept a target list")
        command.append("--workspace")
    else:
        raise SelectionError(f"unsupported mutation scope: {scope}")

    return [*command, *FIXED_ARGUMENTS]


def load_workspace_package_roots(repo_root: Path) -> dict[str, Path]:
    """Read the current workspace package names and roots from Cargo."""
    try:
        result = subprocess.run(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
            cwd=repo_root,
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as error:
        raise SelectionError("failed to start cargo metadata") from error
    if result.returncode != 0:
        raise SelectionError("cargo metadata failed")

    try:
        metadata = json.loads(result.stdout)
        workspace_members = set(metadata["workspace_members"])
        packages = metadata["packages"]
    except (json.JSONDecodeError, KeyError, TypeError) as error:
        raise SelectionError("cargo metadata returned an invalid workspace") from error
    if not isinstance(packages, list) or not all(
        isinstance(member, str) for member in workspace_members
    ):
        raise SelectionError("cargo metadata returned an invalid workspace")

    roots: dict[str, Path] = {}
    for package in packages:
        if not isinstance(package, dict) or package.get("id") not in workspace_members:
            continue
        name = package.get("name")
        manifest_path = package.get("manifest_path")
        if not isinstance(name, str) or not name or not isinstance(manifest_path, str):
            raise SelectionError("cargo metadata returned an invalid package")
        try:
            package_root = Path(manifest_path).resolve(strict=True).parent
            package_root.relative_to(repo_root.resolve(strict=True))
        except (OSError, ValueError) as error:
            raise SelectionError("workspace package must remain inside the repository") from error
        if name in roots:
            raise SelectionError(f"duplicate workspace package name: {name}")
        roots[name] = package_root

    if set(roots) == set() or set(roots) != {
        package["name"]
        for package in packages
        if isinstance(package, dict) and package.get("id") in workspace_members
    }:
        raise SelectionError("cargo metadata did not resolve every workspace package")
    return roots


def _verify_nonempty_selection(command: list[str], repo_root: Path) -> int | None:
    separator = command.index("--")
    selection_arguments = command[:separator]
    list_command = [*selection_arguments, "--list", "--json"]
    try:
        result = subprocess.run(
            list_command,
            cwd=repo_root,
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as error:
        print(f"failed to list the selected mutants: {error}", file=sys.stderr)
        return 1
    if result.returncode != 0:
        print("cargo-mutants could not validate the explicit selection", file=sys.stderr)
        return result.returncode
    try:
        mutants = json.loads(result.stdout)
    except json.JSONDecodeError:
        print("cargo-mutants returned an invalid selection list", file=sys.stderr)
        return 2
    if not isinstance(mutants, list) or not mutants:
        print("explicit mutation selection contains no mutants", file=sys.stderr)
        return 2

    selected_files = {
        selection_arguments[index + 1]
        for index, argument in enumerate(selection_arguments[:-1])
        if argument == "--file"
    }
    selected_packages = {
        selection_arguments[index + 1]
        for index, argument in enumerate(selection_arguments[:-1])
        if argument == "--package"
    }
    if any(not isinstance(mutant, dict) for mutant in mutants):
        print("cargo-mutants returned an invalid selection list", file=sys.stderr)
        return 2
    observed_files = {mutant.get("file") for mutant in mutants}
    observed_packages = {mutant.get("package") for mutant in mutants}
    if not selected_files.issubset(observed_files) or not selected_packages.issubset(
        observed_packages
    ):
        print("at least one explicit mutation target contains no mutants", file=sys.stderr)
        return 2
    return None


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scope", required=True, choices=SCOPES)
    parser.add_argument("--targets", default="")
    arguments = parser.parse_args(argv)

    try:
        package_roots = load_workspace_package_roots(REPO_ROOT)
        command = build_mutation_command(
            REPO_ROOT,
            arguments.scope,
            arguments.targets,
            package_roots,
        )
    except SelectionError as error:
        print(f"invalid mutation selection: {error}", file=sys.stderr)
        return 2

    selection_failure = _verify_nonempty_selection(command, REPO_ROOT)
    if selection_failure is not None:
        return selection_failure
    return subprocess.run(command, cwd=REPO_ROOT, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
