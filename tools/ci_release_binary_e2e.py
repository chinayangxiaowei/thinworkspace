#!/usr/bin/env python3
"""Exercise the release binary on a disposable GitHub-hosted macOS runner."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parent.parent
BINARY = REPO_ROOT / "target/release/thinws"


def run_command(*arguments: str) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run(
        [str(BINARY), *arguments],
        cwd=REPO_ROOT,
        capture_output=True,
        check=False,
        timeout=60,
    )
    if result.returncode != 0 or result.stderr:
        raise RuntimeError(
            f"thinws {arguments!r} returned {result.returncode}: "
            f"stdout={result.stdout!r}, stderr={result.stderr!r}"
        )
    return result


def run_json(*arguments: str) -> dict[str, Any]:
    document = json.loads(run_command("--json", *arguments).stdout)
    if document.get("schema_version") != 1 or document.get("ok") is not True:
        raise RuntimeError(f"unexpected success envelope: {document!r}")
    if not isinstance(document.get("data"), dict):
        raise RuntimeError(f"success envelope has no data object: {document!r}")
    return document["data"]


def require(value: bool, message: str) -> None:
    if not value:
        raise RuntimeError(message)


def main() -> int:
    # GITHUB_ACTIONS is also true on self-hosted runners, whose home persists.
    if (
        sys.platform != "darwin"
        or os.environ.get("GITHUB_ACTIONS") != "true"
        or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted"
    ):
        raise RuntimeError("release binary E2E requires a GitHub-hosted macOS runner")
    runner_temp = Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)
    config = Path.home() / "Library/Application Support/ThinWorkspace/config.toml"
    if config.exists() or config.is_symlink():
        raise RuntimeError(f"refusing to replace an existing bootstrap config: {config}")
    require(BINARY.is_file(), f"release binary is missing: {BINARY}")

    # The hosted runner is disposable. Keep this exact test root on failure so
    # subsequent diagnostic steps in the same job can inspect it.
    test_root = Path(tempfile.mkdtemp(prefix="thinws-release-e2e-", dir=runner_temp))
    source = test_root / "source"
    source.mkdir()
    (source / "note.txt").write_bytes(b"source content")
    data_root = test_root / "data-root"
    print(f"release CLI test root: {test_root}", flush=True)

    initialized = run_json("init", "--data-root", str(data_root))
    require(initialized.get("result") == "initialized", "init did not create a new instance")
    require(initialized.get("data_root") == str(data_root), "init changed the data root")
    doctor = run_json("doctor")
    require(doctor.get("status") == "ready", "doctor did not report a ready instance")

    created = run_json(
        "workspace", "create", "--source", str(source), "--name", "release-e2e"
    )
    require(created.get("result") == "created", "workspace was not newly created")
    materialization = created.get("materialization")
    require(isinstance(materialization, dict), "create omitted materialization evidence")
    require(materialization.get("actual_mode") == "cow-clone", "create was not CoW")
    require(materialization.get("cow") == "confirmed", "CoW was not confirmed")
    workspace = Path(created["path"])
    require(
        workspace.resolve(strict=True).is_relative_to(data_root.resolve(strict=True)),
        "workspace path escaped the test data root",
    )
    require((workspace / "note.txt").read_bytes() == b"source content", "copy content differs")

    path_output = run_command("workspace", "path", "release-e2e").stdout
    require(path_output == os.fsencode(workspace) + b"\n", "workspace path bytes differ")
    (workspace / "note.txt").write_bytes(b"workspace content")
    require((source / "note.txt").read_bytes() == b"source content", "clone changed source")

    removed = run_json("workspace", "remove", "release-e2e")
    require(removed.get("result") == "removed", "workspace remove did not complete")
    require(removed.get("forced") is False, "ordinary remove was marked forced")
    require(not workspace.exists(), "workspace path remained after remove")
    require((source / "note.txt").read_bytes() == b"source content", "remove changed source")
    print("release binary init/create/path/remove E2E passed", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
