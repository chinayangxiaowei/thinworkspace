from __future__ import annotations

import plistlib
import subprocess
import unittest
from unittest import mock

from tools import ci_detach_image


RUNNER_TEMP = "/private/tmp/thinws-ci-runner"
IMAGE_PATH = f"{RUNNER_TEMP}/thinws-p0-cross-volume.dmg"
MOUNTPOINT = f"{RUNNER_TEMP}/thinws-p0-submount-source/mounted"
DEVICE = "/dev/disk9"
HDIUTIL = "/usr/bin/hdiutil"


def command_result(returncode: int, stdout: bytes = b"", stderr: bytes = b""):
    return subprocess.CompletedProcess([], returncode, stdout, stderr)


def entity(device: str, *, partition_scheme: bool = False, mounted: bool = False):
    value = {"dev-entry": device}
    if partition_scheme:
        value["content-hint"] = "GUID_partition_scheme"
    if mounted:
        value["mount-point"] = MOUNTPOINT
    return value


def image_record(
    image_path: str = IMAGE_PATH,
    entities: list[dict[str, object]] | None = None,
):
    return {
        "image-path": image_path,
        "system-entities": entities
        if entities is not None
        else [
            entity(DEVICE, partition_scheme=True),
            entity(f"{DEVICE}s1"),
            entity("/dev/disk10"),
            entity("/dev/disk10s1", mounted=True),
        ],
    }


def info_payload(*images: dict[str, object]) -> bytes:
    return plistlib.dumps({"images": list(images)})


class DetachImageTests(unittest.TestCase):
    def setUp(self):
        run_patch = mock.patch.object(ci_detach_image.subprocess, "run")
        sleep_patch = mock.patch.object(ci_detach_image.time, "sleep")
        self.addCleanup(run_patch.stop)
        self.addCleanup(sleep_patch.stop)
        self.run = run_patch.start()
        self.sleep = sleep_patch.start()

    def test_busy_retry_re_resolves_image_to_device_after_mountpoint_disappears(self):
        info_count = 0
        detach_targets: list[str] = []
        old_mountpoint_attempts = 0

        def simulate_partial_detach(argv, **kwargs):
            nonlocal info_count, old_mountpoint_attempts
            self.assertFalse(kwargs.get("shell", False))
            if argv == [HDIUTIL, "info", "-plist"]:
                states = [
                    info_payload(image_record()),
                    info_payload(
                        image_record(
                            entities=[entity(DEVICE, partition_scheme=True)]
                        )
                    ),
                    info_payload(),
                ]
                result = command_result(0, states[info_count])
                info_count += 1
                return result
            if argv[:2] == [HDIUTIL, "detach"]:
                target = argv[2]
                detach_targets.append(target)
                if target == MOUNTPOINT:
                    old_mountpoint_attempts += 1
                    return command_result(
                        16 if old_mountpoint_attempts == 1 else 1,
                        stderr=b"mountpoint no longer exists",
                    )
                return command_result(16 if len(detach_targets) == 1 else 0)
            self.fail(f"unexpected command: {argv!r}")

        self.run.side_effect = simulate_partial_detach

        ci_detach_image.detach_image(IMAGE_PATH, RUNNER_TEMP)

        self.assertEqual(detach_targets, [DEVICE, DEVICE])
        self.assertEqual(info_count, 3)
        self.sleep.assert_called_once_with(1)

    def test_normal_detach_uses_partition_scheme_device_and_confirms_absence(self):
        self.run.side_effect = [
            command_result(0, info_payload(image_record())),
            command_result(0),
            command_result(0, info_payload()),
        ]

        ci_detach_image.detach_image(IMAGE_PATH, RUNNER_TEMP)

        self.assertEqual(
            self.run.call_args_list,
            [
                mock.call(
                    [HDIUTIL, "info", "-plist"],
                    check=False,
                    capture_output=True,
                    timeout=10,
                ),
                mock.call(
                    [HDIUTIL, "detach", DEVICE],
                    check=False,
                    capture_output=True,
                    timeout=30,
                ),
                mock.call(
                    [HDIUTIL, "info", "-plist"],
                    check=False,
                    capture_output=True,
                    timeout=10,
                ),
            ],
        )
        self.sleep.assert_not_called()

    def test_no_matching_image_is_confirmed_without_detaching_any_other_image(self):
        unrelated = image_record("/private/tmp/unrelated.dmg")
        self.run.return_value = command_result(0, info_payload(unrelated))

        ci_detach_image.detach_image(IMAGE_PATH, RUNNER_TEMP)

        self.run.assert_called_once_with(
            [HDIUTIL, "info", "-plist"],
            check=False,
            capture_output=True,
            timeout=10,
        )
        self.sleep.assert_not_called()

    def test_non_busy_detach_error_is_not_retried(self):
        self.run.side_effect = [
            command_result(0, info_payload(image_record())),
            command_result(5, stderr=b"non-busy failure"),
        ]

        with self.assertRaisesRegex(
            ci_detach_image.DetachError, "exit 5: non-busy failure"
        ):
            ci_detach_image.detach_image(IMAGE_PATH, RUNNER_TEMP)

        self.assertEqual(self.run.call_count, 2)
        self.sleep.assert_not_called()

    def test_query_failure_is_not_treated_as_detached(self):
        self.run.return_value = command_result(7, stderr=b"query failed")

        with self.assertRaisesRegex(
            ci_detach_image.DetachError, "info.*exit 7: query failed"
        ):
            ci_detach_image.detach_image(IMAGE_PATH, RUNNER_TEMP)

        self.run.assert_called_once()
        self.sleep.assert_not_called()

    def test_malformed_info_plist_is_rejected(self):
        malformed_payloads = [
            b"not a plist",
            b'<?xml version="1.0"?><plist><dict>',
            plistlib.dumps([]),
            plistlib.dumps({}),
            plistlib.dumps({"images": "not-an-array"}),
            plistlib.dumps({"images": ["not-a-record"]}),
        ]
        for payload in malformed_payloads:
            with self.subTest(payload=payload[:20]):
                self.run.reset_mock(side_effect=True, return_value=True)
                self.run.return_value = command_result(0, payload)
                with self.assertRaises(ci_detach_image.DetachError):
                    ci_detach_image.detach_image(IMAGE_PATH, RUNNER_TEMP)
                self.run.assert_called_once()

    def test_invalid_or_inconsistent_paths_fail_before_any_command(self):
        invalid_paths = [
            ("", RUNNER_TEMP),
            ("/", RUNNER_TEMP),
            (IMAGE_PATH, ""),
            (IMAGE_PATH, "/"),
            ("relative.dmg", RUNNER_TEMP),
            (IMAGE_PATH, "relative-runner"),
            (f"{RUNNER_TEMP}/other.dmg", RUNNER_TEMP),
            (IMAGE_PATH, "/private/tmp/other-runner"),
        ]
        for image_path, runner_temp in invalid_paths:
            with self.subTest(image_path=image_path, runner_temp=runner_temp):
                with self.assertRaises(ci_detach_image.DetachError):
                    ci_detach_image.detach_image(image_path, runner_temp)
        self.run.assert_not_called()
        self.sleep.assert_not_called()

    def test_duplicate_image_or_partition_scheme_association_is_rejected(self):
        ambiguous_payloads = [
            info_payload(image_record(), image_record()),
            info_payload(
                image_record(
                    entities=[
                        entity(DEVICE, partition_scheme=True),
                        entity("/dev/disk11", partition_scheme=True),
                    ]
                )
            ),
            info_payload(
                image_record(
                    entities=[
                        entity(DEVICE, partition_scheme=True),
                        entity(DEVICE, partition_scheme=True),
                    ]
                )
            ),
        ]
        for payload in ambiguous_payloads:
            with self.subTest():
                self.run.reset_mock(side_effect=True, return_value=True)
                self.run.return_value = command_result(0, payload)
                with self.assertRaises(ci_detach_image.DetachError):
                    ci_detach_image.detach_image(IMAGE_PATH, RUNNER_TEMP)
                self.run.assert_called_once()

    def test_missing_or_invalid_partition_scheme_device_is_rejected(self):
        invalid_entities = [
            [],
            [entity(f"{DEVICE}s1")],
            [{"content-hint": "GUID_partition_scheme"}],
            [{"content-hint": "GUID_partition_scheme", "dev-entry": 9}],
            [entity(f"{DEVICE}s1", partition_scheme=True)],
            [entity("/dev/rdisk9", partition_scheme=True)],
        ]
        for entities in invalid_entities:
            with self.subTest(entities=entities):
                self.run.reset_mock(side_effect=True, return_value=True)
                self.run.return_value = command_result(
                    0, info_payload(image_record(entities=entities))
                )
                with self.assertRaises(ci_detach_image.DetachError):
                    ci_detach_image.detach_image(IMAGE_PATH, RUNNER_TEMP)
                self.run.assert_called_once()

    def test_busy_retry_is_bounded_to_five_detach_attempts(self):
        results = []
        for _ in range(5):
            results.extend(
                [
                    command_result(0, info_payload(image_record())),
                    command_result(16, stderr=b"busy"),
                ]
            )
        self.run.side_effect = results

        with self.assertRaisesRegex(ci_detach_image.DetachError, "busy.*5 attempts"):
            ci_detach_image.detach_image(IMAGE_PATH, RUNNER_TEMP)

        self.assertEqual(self.run.call_count, 10)
        self.assertEqual(
            self.sleep.call_args_list,
            [mock.call(1), mock.call(2), mock.call(4), mock.call(8)],
        )

    def test_success_exit_still_requires_image_absence_confirmation(self):
        self.run.side_effect = [
            command_result(0, info_payload(image_record())),
            command_result(0),
            command_result(0, info_payload(image_record())),
        ]

        with self.assertRaisesRegex(ci_detach_image.DetachError, "still attached"):
            ci_detach_image.detach_image(IMAGE_PATH, RUNNER_TEMP)

        self.assertEqual(self.run.call_count, 3)
        self.sleep.assert_not_called()


if __name__ == "__main__":
    unittest.main()
