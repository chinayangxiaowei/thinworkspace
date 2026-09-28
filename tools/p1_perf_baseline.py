#!/usr/bin/env python3
"""Measure release CLI CoW creation on an isolated, repeatable APFS fixture.

This records a local baseline, not a product latency promise or a disk-space
guarantee. Run it without concurrent mutation, fuzz, or other heavy I/O.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import plistlib
import statistics
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


SMALL_BYTES = 4096
SMALL_COUNT = 256
LARGE_COUNT = 8
LARGE_BYTES = 8 * 1024 * 1024
ROUNDS = 3
COPIES_PER_ROUND = 10


def make_source(
    root: Path,
    *,
    small_count: int = SMALL_COUNT,
    large_count: int = LARGE_COUNT,
    large_bytes: int = LARGE_BYTES,
) -> dict[str, Any]:
    """Create the same ordinary-file fixture on every run and hash its manifest."""
    root.mkdir(parents=True, exist_ok=True)
    small = root / "small"
    large = root / "large"
    small.mkdir()
    large.mkdir()
    digest = hashlib.sha256()
    logical_bytes = 0
    for index in range(small_count):
        relative = f"small/file-{index:04d}.bin"
        content = hashlib.sha256(relative.encode()).digest() * (SMALL_BYTES // 32)
        (root / relative).write_bytes(content)
        digest.update(relative.encode() + b"\0" + hashlib.sha256(content).digest())
        logical_bytes += len(content)
    for index in range(large_count):
        relative = f"large/file-{index:04d}.bin"
        block = hashlib.sha256(relative.encode()).digest()
        content = (block * math.ceil(large_bytes / len(block)))[:large_bytes]
        (root / relative).write_bytes(content)
        digest.update(relative.encode() + b"\0" + hashlib.sha256(content).digest())
        logical_bytes += len(content)
    return {
        "file_count": small_count + large_count,
        "logical_bytes": logical_bytes,
        "manifest_sha256": digest.hexdigest(),
    }


def summarize_ms(values: list[float]) -> dict[str, float | int]:
    """Use observed nearest-rank p95; do not interpolate a latency claim."""
    if not values:
        raise ValueError("at least one observation is required")
    ordered = sorted(values)
    return {
        "count": len(values),
        "min_ms": round(ordered[0], 3),
        "median_ms": round(statistics.median(ordered), 3),
        "p95_ms": round(ordered[math.ceil(0.95 * len(ordered)) - 1], 3),
        "max_ms": round(ordered[-1], 3),
    }


def volume_info(root: Path) -> dict[str, Any]:
    result = subprocess.run(
        ["/usr/sbin/diskutil", "info", "-plist", str(root)],
        capture_output=True,
        check=True,
        timeout=30,
    )
    info = plistlib.loads(result.stdout)
    if info.get("FilesystemType") != "apfs" or not info.get("VolumeUUID"):
        raise ValueError("benchmark root must be a mounted APFS volume")
    return {
        "volume_uuid": info["VolumeUUID"],
        "mount_point": info.get("MountPoint"),
        "bus_protocol": info.get("BusProtocol"),
        "container_free_bytes": info.get("APFSContainerFree"),
    }


def run_json(binary: Path, home: Path, *arguments: str) -> dict[str, Any]:
    environment = os.environ.copy()
    environment["HOME"] = str(home)
    result = subprocess.run(
        [str(binary), "--json", *arguments],
        cwd=home,
        env=environment,
        capture_output=True,
        check=False,
        timeout=180,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"thinws {arguments!r} failed: exit={result.returncode}, "
            f"stdout={result.stdout!r}, stderr={result.stderr!r}"
        )
    document = json.loads(result.stdout)
    if document.get("schema_version") != 1 or document.get("ok") is not True:
        raise RuntimeError(f"unexpected CLI envelope: {document!r}")
    data = document.get("data")
    if not isinstance(data, dict):
        raise RuntimeError("CLI success envelope has no data object")
    return data


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--volume-root", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    volume_root = args.volume_root.resolve(strict=True)
    if not binary.is_file() or not volume_root.is_dir():
        raise ValueError("binary and volume root must exist")
    output = args.output.absolute()
    if not output.parent.is_dir() or output.exists() or output.is_symlink():
        raise ValueError("output parent must exist and output must not already exist")
    initial = volume_info(volume_root)
    with tempfile.TemporaryDirectory(prefix="thinws-p1-perf-", dir=volume_root) as directory:
        scratch = Path(directory)
        home = scratch / "home"
        home.mkdir()
        source = scratch / "source"
        fixture = make_source(source)
        target_root = scratch / "targets"
        target_root.mkdir()
        initialized = run_json(binary, home, "init")
        if initialized.get("result") != "initialized":
            raise RuntimeError("isolated benchmark instance was not initialized")
        if initialized.get("control_root") != str(home / ".thinws"):
            raise RuntimeError("isolated benchmark control root is not the requested HOME")
        before_copies = volume_info(volume_root)
        if before_copies["volume_uuid"] != initial["volume_uuid"]:
            raise RuntimeError("APFS volume identity changed during fixture setup")
        observations: list[dict[str, int | float]] = []
        for round_index in range(ROUNDS):
            for copy_index in range(COPIES_PER_ROUND):
                name = f"bench-r{round_index:02d}-c{copy_index:02d}"
                target = target_root / name
                started = time.perf_counter_ns()
                created = run_json(
                    binary, home, "workspace", "create", "--source", str(source),
                    "--target", str(target), "--name", name
                )
                elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
                materialization = created.get("materialization")
                if (
                    created.get("result") != "created"
                    or not isinstance(materialization, dict)
                    or materialization.get("actual_mode") != "cow-clone"
                    or materialization.get("cow") != "confirmed"
                ):
                    raise RuntimeError(f"copy {name} did not confirm CoW: {created!r}")
                workspace = Path(created["path"]).resolve(strict=True)
                if workspace != target.resolve(strict=True):
                    raise RuntimeError(f"copy {name} did not use its explicit target")
                observations.append(
                    {"round": round_index + 1, "copy": copy_index + 1, "elapsed_ms": round(elapsed_ms, 3)}
                )
                print(f"{name}: {elapsed_ms:.3f} ms", file=sys.stderr, flush=True)
        after = volume_info(volume_root)
        if after["volume_uuid"] != initial["volume_uuid"]:
            raise RuntimeError("APFS volume identity changed during benchmark")
        measurements = [float(item["elapsed_ms"]) for item in observations]
        result = {
            "schema_version": 1,
            "measured_at_utc": datetime.now(timezone.utc).isoformat(),
            "binary_sha256": sha256_file(binary),
            "host": {"platform": platform.platform(), "architecture": platform.machine()},
            "volume": initial,
            "fixture": fixture,
            "rounds": ROUNDS,
            "copies_per_round": COPIES_PER_ROUND,
            "observations": observations,
            "summary_ms": summarize_ms(measurements),
            "container_free_before_copies_bytes": before_copies["container_free_bytes"],
            "container_free_after_copies_bytes": after["container_free_bytes"],
            "space_delta_is_advisory": True,
        }
    with output.open("x", encoding="utf-8") as destination:
        json.dump(result, destination, ensure_ascii=False, indent=2)
        destination.write("\n")
    print(f"wrote {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
