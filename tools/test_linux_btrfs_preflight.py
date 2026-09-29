from __future__ import annotations

import io
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest import mock

from tools import linux_btrfs_preflight


class LinuxBtrfsTestRootTests(unittest.TestCase):
    def test_missing_environment_variable_never_selects_a_default_directory(self) -> None:
        with self.assertRaisesRegex(
            linux_btrfs_preflight.TestRootError,
            "THINWS_LINUX_BTRFS_TEST_ROOT is required",
        ):
            linux_btrfs_preflight.configured_root({})

    def test_relative_directory_is_rejected_before_filesystem_probe(self) -> None:
        with mock.patch.object(linux_btrfs_preflight, "filesystem_type") as probe:
            with self.assertRaisesRegex(
                linux_btrfs_preflight.TestRootError, "absolute"
            ):
                linux_btrfs_preflight.configured_root(
                    {linux_btrfs_preflight.ROOT_ENV: "relative/test-root"}
                )
            probe.assert_not_called()

    def test_non_btrfs_directory_is_rejected_without_writing_into_it(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            with mock.patch.object(
                linux_btrfs_preflight, "filesystem_type", return_value="ext4"
            ):
                with self.assertRaisesRegex(
                    linux_btrfs_preflight.TestRootError, "expected btrfs"
                ):
                    linux_btrfs_preflight.configured_root(
                        {linux_btrfs_preflight.ROOT_ENV: str(root)}
                    )
            self.assertEqual(list(root.iterdir()), [])

    def test_symlinked_directory_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary).resolve()
            actual = parent / "actual"
            actual.mkdir()
            alias = parent / "alias"
            alias.symlink_to(actual, target_is_directory=True)
            with mock.patch.object(linux_btrfs_preflight, "filesystem_type") as probe:
                with self.assertRaisesRegex(
                    linux_btrfs_preflight.TestRootError, "symlink"
                ):
                    linux_btrfs_preflight.configured_root(
                        {linux_btrfs_preflight.ROOT_ENV: str(alias)}
                    )
                probe.assert_not_called()

    def test_valid_btrfs_directory_is_returned_without_creating_files(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            with mock.patch.object(
                linux_btrfs_preflight, "filesystem_type", return_value="btrfs"
            ):
                self.assertEqual(
                    linux_btrfs_preflight.configured_root(
                        {linux_btrfs_preflight.ROOT_ENV: str(root)}
                    ),
                    root,
                )
            self.assertEqual(list(root.iterdir()), [])

    def test_unwritable_test_root_reports_failure_without_a_traceback(self) -> None:
        output = io.StringIO()
        with (
            mock.patch.object(linux_btrfs_preflight.sys, "platform", "linux"),
            mock.patch.object(
                linux_btrfs_preflight, "configured_root", return_value=Path("/test")
            ),
            mock.patch.object(
                linux_btrfs_preflight,
                "run_reflink_smoke",
                side_effect=PermissionError("test root is not writable"),
            ),
            redirect_stderr(output),
        ):
            self.assertEqual(linux_btrfs_preflight.main(), 1)
        self.assertIn("not writable", output.getvalue())
        self.assertNotIn("Traceback", output.getvalue())


if __name__ == "__main__":
    unittest.main()
