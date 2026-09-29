#!/usr/bin/env python3
"""Run every declared target applicable to this host with the release budget."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
BUDGET_SECONDS = 300
PROCESS_TIMEOUT_SECONDS = BUDGET_SECONDS + 120


def target_names(manifest_path: Path) -> list[str]:
    manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    names = [binary["name"] for binary in manifest["bin"]]
    if not names or len(names) != len(set(names)):
        raise ValueError("missing or duplicate fuzz target in manifest")
    return names


def eligible_target_names(manifest_path: Path, platform: str) -> list[str]:
    if platform not in {"darwin", "linux"}:
        raise ValueError(f"unsupported fuzz platform: {platform}")
    names = target_names(manifest_path)
    manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    macos_only = (
        manifest.get("package", {})
        .get("metadata", {})
        .get("thinws", {})
        .get("macos_only_fuzz_targets")
    )
    if not isinstance(macos_only, list) or not all(
        isinstance(name, str) for name in macos_only
    ):
        raise ValueError("missing macOS-only fuzz target classification")
    if len(macos_only) != len(set(macos_only)):
        raise ValueError("duplicate macOS-only fuzz target")
    if unknown := set(macos_only) - set(names):
        raise ValueError(f"unknown macOS-only fuzz target: {sorted(unknown)}")
    if platform == "darwin":
        return names
    return [name for name in names if name not in macos_only]


def fuzz_command(target: str, nightly: str, corpus: Path) -> list[str]:
    max_len = 65537 if target == "thinws_bootstrap_document" else 4096
    return [
        "cargo",
        f"+{nightly}",
        "fuzz",
        "run",
        target,
        str(corpus),
        "--",
        f"-max_total_time={BUDGET_SECONDS}",
        "-timeout=5",
        f"-max_len={max_len}",
        "-print_final_stats=1",
        "-verbosity=0",
    ]


def record_result(target: str, result: str) -> None:
    line = f"| `{target}` | {BUDGET_SECONDS}s | {result} |\n"
    print(line.strip(), flush=True)
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(summary).open("a", encoding="utf-8") as output:
            output.write(line)


def prepare_corpus(target: str, scratch: Path) -> Path:
    corpus = scratch / target
    corpus.mkdir()
    seeds = REPO_ROOT / "fuzz/corpus" / target
    if seeds.exists() or seeds.is_symlink():
        if not seeds.is_dir() or seeds.is_symlink():
            raise ValueError(f"fuzz seeds must be a directory: {target}")
        for seed in seeds.iterdir():
            if not seed.is_file() or seed.is_symlink():
                raise ValueError(f"fuzz seed must be a regular file: {target}")
            shutil.copy2(seed, corpus / seed.name)
    return corpus


def main() -> int:
    nightly = os.environ["THINWS_FUZZ_NIGHTLY"]
    targets = eligible_target_names(REPO_ROOT / "fuzz/Cargo.toml", sys.platform)
    print(
        f"Running {len(targets)} applicable fuzz targets with {BUDGET_SECONDS}s each",
        flush=True,
    )
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(summary).open("a", encoding="utf-8") as output:
            output.write(
                f"### Phase 1 release fuzz ({nightly}, {sys.platform})\n\n"
                "| Target | Budget | Result |\n|---|---:|---|\n"
            )
    with tempfile.TemporaryDirectory(prefix="thinws-release-fuzz-") as directory:
        scratch = Path(directory)
        for target in targets:
            corpus = prepare_corpus(target, scratch)
            print(f"Starting {target}", flush=True)
            try:
                result = subprocess.run(
                    fuzz_command(target, nightly, corpus),
                    cwd=REPO_ROOT,
                    check=False,
                    timeout=PROCESS_TIMEOUT_SECONDS,
                )
            except subprocess.TimeoutExpired:
                record_result(target, "process timeout")
                return 1
            if result.returncode != 0:
                record_result(target, f"failed (exit {result.returncode})")
                return 1
            record_result(target, "passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
