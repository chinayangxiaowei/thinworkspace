#!/usr/bin/env python3
"""Check a user-selected Linux Btrfs test root with a disposable reflink pair."""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT_ENV = "THINWS_LINUX_BTRFS_TEST_ROOT"


class TestRootError(RuntimeError):
    """The configured real-filesystem test root is not usable."""


def filesystem_type(root: Path) -> str:
    try:
        result = subprocess.run(
            ["findmnt", "--noheadings", "--output", "FSTYPE", "--target", str(root)],
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise TestRootError("cannot inspect the configured filesystem") from error
    kind = result.stdout.strip()
    if result.returncode != 0 or not kind or "\n" in kind:
        raise TestRootError("cannot identify the configured filesystem")
    return kind


def configured_root(environment=None) -> Path:
    environment = os.environ if environment is None else environment
    raw = environment.get(ROOT_ENV)
    if not raw:
        raise TestRootError(f"{ROOT_ENV} is required; no default test directory exists")
    if "\0" in raw or not Path(raw).is_absolute() or os.path.normpath(raw) != raw:
        raise TestRootError(f"{ROOT_ENV} must be a normalized absolute directory")
    root = Path(raw)
    if root == Path("/"):
        raise TestRootError(f"{ROOT_ENV} must not be a filesystem root")
    try:
        resolved = root.resolve(strict=True)
    except (OSError, RuntimeError) as error:
        raise TestRootError(f"{ROOT_ENV} must name an existing directory") from error
    if resolved != root:
        raise TestRootError(f"{ROOT_ENV} must not contain a symlink")
    if not root.is_dir():
        raise TestRootError(f"{ROOT_ENV} must name an existing directory")
    kind = filesystem_type(root)
    if kind != "btrfs":
        raise TestRootError(f"expected btrfs at {ROOT_ENV}, found {kind}")
    return root


def run_reflink_smoke(root: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="thinws-btrfs-test-", dir=root) as temporary:
        source = Path(temporary) / "source"
        clone = Path(temporary) / "clone"
        original = b"thinws btrfs reflink probe\n"
        source.write_bytes(original)
        try:
            result = subprocess.run(
                ["cp", "--reflink=always", "--", str(source), str(clone)],
                check=False,
                capture_output=True,
                text=True,
                timeout=30,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise TestRootError("reflink probe could not run") from error
        if result.returncode != 0:
            raise TestRootError("reflink probe failed; the test root cannot provide CoW")
        if source.stat().st_dev != clone.stat().st_dev or clone.read_bytes() != original:
            raise TestRootError("reflink probe produced an invalid copy")
        clone.write_bytes(b"changed clone\n")
        if source.read_bytes() != original:
            raise TestRootError("reflink probe did not isolate subsequent writes")


def main() -> int:
    if sys.platform != "linux":
        print("Linux Btrfs preflight requires Linux", file=sys.stderr)
        return 1
    try:
        root = configured_root()
        run_reflink_smoke(root)
    except (TestRootError, OSError) as error:
        print(error, file=sys.stderr)
        return 1
    print(f"Btrfs reflink confirmed under {root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
