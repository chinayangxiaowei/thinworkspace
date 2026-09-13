#!/usr/bin/env python3
"""Safely detach the one disk image created by the P0 macOS CI job."""

from __future__ import annotations

import argparse
import os
import plistlib
import re
import subprocess
import sys
import time
from pathlib import Path
from xml.parsers.expat import ExpatError


IMAGE_NAME = "thinws-p0-cross-volume.dmg"
HDIUTIL = "/usr/bin/hdiutil"
PARTITION_SCHEME_HINT = "GUID_partition_scheme"
BUSY_EXIT = 16
INFO_TIMEOUT_SECONDS = 10
DETACH_TIMEOUT_SECONDS = 30
RETRY_DELAYS_SECONDS = (1, 2, 4, 8)
WHOLE_DISK_DEVICE = re.compile(r"/dev/disk[1-9][0-9]*\Z")


class DetachError(RuntimeError):
    """The CI image could not be safely confirmed detached."""


def _validated_image_path(image_path: str, runner_temp: str) -> str:
    for label, value in (("image", image_path), ("runner temp", runner_temp)):
        if not value or "\0" in value:
            raise DetachError(f"{label} path must not be empty")
        if not Path(value).is_absolute() or os.path.normpath(value) != value:
            raise DetachError(f"{label} path must be normalized and absolute")

    runner = Path(runner_temp)
    image = Path(image_path)
    if runner == Path("/") or image == Path("/"):
        raise DetachError("refusing a filesystem root path")
    expected = runner / IMAGE_NAME
    if image != expected:
        raise DetachError(f"image path must be exactly {expected}")
    return str(expected)


def _run_hdiutil(argv: list[str], timeout: int) -> subprocess.CompletedProcess[bytes]:
    try:
        return subprocess.run(
            argv,
            check=False,
            capture_output=True,
            timeout=timeout,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise DetachError(f"failed to run {argv[1]}: {error}") from error


def _command_error(verb: str, result: subprocess.CompletedProcess[bytes]) -> DetachError:
    stderr = result.stderr.decode("utf-8", errors="replace").strip()
    if len(stderr) > 1000:
        stderr = f"{stderr[:1000]}..."
    detail = f": {stderr}" if stderr else ""
    return DetachError(f"hdiutil {verb} failed with exit {result.returncode}{detail}")


def _partition_scheme_device(info: bytes, image_path: str) -> str | None:
    try:
        root = plistlib.loads(info)
    except (
        plistlib.InvalidFileException,
        ExpatError,
        ValueError,
        TypeError,
        OverflowError,
    ) as error:
        raise DetachError(f"hdiutil info returned an invalid plist: {error}") from error
    if not isinstance(root, dict) or "images" not in root:
        raise DetachError("hdiutil info plist is missing the images array")
    images = root["images"]
    if not isinstance(images, list):
        raise DetachError("hdiutil info images value is not an array")

    matches = []
    for image in images:
        if not isinstance(image, dict):
            raise DetachError("hdiutil info contains a malformed image record")
        observed_path = image.get("image-path")
        if not isinstance(observed_path, str) or not observed_path:
            raise DetachError("hdiutil info image record has an invalid image path")
        if observed_path == image_path:
            matches.append(image)
    if not matches:
        return None
    if len(matches) != 1:
        raise DetachError("hdiutil info contains duplicate associations for the CI image")

    entities = matches[0].get("system-entities")
    if not isinstance(entities, list):
        raise DetachError("CI image association is missing the system entities array")
    partition_schemes = []
    for entity in entities:
        if not isinstance(entity, dict):
            raise DetachError("CI image association contains a malformed system entity")
        content_hint = entity.get("content-hint")
        if content_hint is not None and not isinstance(content_hint, str):
            raise DetachError("CI image system entity has an invalid content hint")
        if content_hint == PARTITION_SCHEME_HINT:
            partition_schemes.append(entity)
    if len(partition_schemes) != 1:
        raise DetachError(
            "CI image must have exactly one GUID partition scheme association"
        )

    device = partition_schemes[0].get("dev-entry")
    if not isinstance(device, str) or not WHOLE_DISK_DEVICE.fullmatch(device):
        raise DetachError("CI image GUID partition scheme has an invalid device")
    return device


def _query_device(image_path: str) -> str | None:
    result = _run_hdiutil(
        [HDIUTIL, "info", "-plist"],
        INFO_TIMEOUT_SECONDS,
    )
    if result.returncode != 0:
        raise _command_error("info", result)
    return _partition_scheme_device(result.stdout, image_path)


def detach_image(image_path: str, runner_temp: str) -> None:
    """Detach the exact P0 image, retrying only a reported busy result."""
    image_path = _validated_image_path(image_path, runner_temp)

    for attempt in range(1, 6):
        device = _query_device(image_path)
        if device is None:
            return

        result = _run_hdiutil(
            [HDIUTIL, "detach", device],
            DETACH_TIMEOUT_SECONDS,
        )
        if result.returncode == 0:
            remaining_device = _query_device(image_path)
            if remaining_device is None:
                return
            raise DetachError(
                "hdiutil detach returned success but the CI image is still attached "
                f"as {remaining_device}"
            )
        if result.returncode != BUSY_EXIT:
            raise _command_error("detach", result)
        if attempt == 5:
            raise DetachError("hdiutil detach remained busy after 5 attempts")

        delay = RETRY_DELAYS_SECONDS[attempt - 1]
        print(
            f"hdiutil detach was busy on attempt {attempt}/5; "
            f"retrying in {delay} second(s)",
            file=sys.stderr,
        )
        time.sleep(delay)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    parser.add_argument("--runner-temp", required=True)
    arguments = parser.parse_args(argv)
    try:
        detach_image(arguments.image, arguments.runner_temp)
    except DetachError as error:
        print(f"failed to detach P0 CI image: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
