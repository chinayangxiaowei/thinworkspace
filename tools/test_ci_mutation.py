from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools import ci_mutation


FIXED_TAIL = [
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


class MutationSelectionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.probe = self.root / "experiments/p0/probe"
        self.cleanup = self.root / "experiments/p0/cleanup"
        for package in (self.probe, self.cleanup):
            (package / "src/nested").mkdir(parents=True)
            (package / "Cargo.toml").write_text("[package]\n", encoding="utf-8")
        self.probe_lib = self.probe / "src/lib.rs"
        self.probe_nested = self.probe / "src/nested/path.rs"
        self.cleanup_lib = self.cleanup / "src/lib.rs"
        for source in (self.probe_lib, self.probe_nested, self.cleanup_lib):
            source.write_text("pub fn production() {}\n", encoding="utf-8")
        self.packages = {
            "thinws-p0-probe": self.probe,
            "thinws-p0-cleanup": self.cleanup,
        }

    def command(self, scope: str, targets: str = "") -> list[str]:
        return ci_mutation.build_mutation_command(
            self.root, scope, targets, self.packages
        )

    def test_modules_generate_one_exact_file_argument_per_production_source(self):
        targets = (
            "experiments/p0/probe/src/lib.rs,"
            "experiments/p0/probe/src/nested/path.rs"
        )

        self.assertEqual(
            self.command("modules", targets),
            [
                "cargo",
                "mutants",
                "--workspace",
                "--file",
                "experiments/p0/probe/src/lib.rs",
                "--file",
                "experiments/p0/probe/src/nested/path.rs",
                *FIXED_TAIL,
            ],
        )

    def test_modules_reject_non_exact_or_escaping_paths(self):
        invalid = [
            "",
            "/tmp/source.rs",
            "../source.rs",
            "experiments/p0/probe/src/../src/lib.rs",
            "experiments/p0/probe/src/*.rs",
            "experiments/p0/probe/src/lib?.rs",
            "experiments/p0/probe/src/[a].rs",
            "experiments\\p0\\probe\\src\\lib.rs",
            "experiments/p0/probe/src/missing.rs",
            "experiments/p0/probe/Cargo.toml",
        ]
        for target in invalid:
            with self.subTest(target=target):
                with self.assertRaises(ci_mutation.SelectionError):
                    self.command("modules", target)

    def test_modules_reject_dedicated_test_sources_and_symlinks(self):
        test_sources = [
            self.probe / "tests/integration.rs",
            self.probe / "src/tests.rs",
            self.probe / "src/test_helper.rs",
            self.probe / "src/helper_test.rs",
            self.probe / "src/helper_tests.rs",
        ]
        for source in test_sources:
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text("#[test] fn only_a_test() {}\n", encoding="utf-8")
        symlink = self.probe / "src/linked.rs"
        symlink.symlink_to(self.probe_lib)

        for source in [*test_sources, symlink]:
            target = source.relative_to(self.root).as_posix()
            with self.subTest(target=target):
                with self.assertRaises(ci_mutation.SelectionError):
                    self.command("modules", target)

    def test_modules_reject_duplicates_and_sources_outside_workspace_packages(self):
        outside = self.root / "tools/source.rs"
        outside.parent.mkdir()
        outside.write_text("fn production() {}\n", encoding="utf-8")

        with self.assertRaises(ci_mutation.SelectionError):
            self.command(
                "modules",
                "experiments/p0/probe/src/lib.rs,"
                "experiments/p0/probe/src/lib.rs",
            )
        with self.assertRaises(ci_mutation.SelectionError):
            self.command("modules", "tools/source.rs")

    def test_crates_accept_only_current_exact_package_names(self):
        self.assertEqual(
            self.command("crates", "thinws-p0-cleanup,thinws-p0-probe"),
            [
                "cargo",
                "mutants",
                "--package",
                "thinws-p0-cleanup",
                "--package",
                "thinws-p0-probe",
                *FIXED_TAIL,
            ],
        )

        for targets in (
            "",
            "thinws-p0-unknown",
            "thinws-p0-probe --workspace",
            "thinws-p0-probe,thinws-p0-probe",
        ):
            with self.subTest(targets=targets):
                with self.assertRaises(ci_mutation.SelectionError):
                    self.command("crates", targets)

    def test_workspace_is_explicit_and_rejects_a_target_list(self):
        self.assertEqual(
            self.command("workspace"),
            ["cargo", "mutants", "--workspace", *FIXED_TAIL],
        )
        with self.assertRaises(ci_mutation.SelectionError):
            self.command("workspace", "thinws-p0-probe")
        with self.assertRaises(ci_mutation.SelectionError):
            self.command("unknown")

    def test_main_executes_the_validated_argv_without_a_shell(self):
        expected = self.command("crates", "thinws-p0-probe")
        list_command = expected[: expected.index("--")] + ["--list", "--json"]
        listed = subprocess.CompletedProcess(
            list_command,
            0,
            json.dumps([{"file": "src/lib.rs", "package": "thinws-p0-probe"}]),
            "",
        )
        result = mock.Mock(returncode=23)
        with (
            mock.patch.object(ci_mutation, "REPO_ROOT", self.root),
            mock.patch.object(
                ci_mutation,
                "load_workspace_package_roots",
                return_value=self.packages,
            ),
            mock.patch.object(
                ci_mutation.subprocess, "run", side_effect=[listed, result]
            ) as run,
        ):
            returncode = ci_mutation.main(
                ["--scope", "crates", "--targets", "thinws-p0-probe"]
            )

        self.assertEqual(returncode, 23)
        self.assertEqual(
            run.call_args_list,
            [
                mock.call(
                    list_command,
                    cwd=self.root,
                    check=False,
                    capture_output=True,
                    text=True,
                ),
                mock.call(expected, cwd=self.root, check=False),
            ],
        )

    def test_main_refuses_an_empty_mutant_selection(self):
        expected = self.command("modules", "experiments/p0/probe/src/lib.rs")
        list_command = expected[: expected.index("--")] + ["--list", "--json"]
        listed = subprocess.CompletedProcess(list_command, 0, "[]", "")
        with (
            mock.patch.object(ci_mutation, "REPO_ROOT", self.root),
            mock.patch.object(
                ci_mutation,
                "load_workspace_package_roots",
                return_value=self.packages,
            ),
            mock.patch.object(
                ci_mutation.subprocess, "run", return_value=listed
            ) as run,
        ):
            returncode = ci_mutation.main(
                [
                    "--scope",
                    "modules",
                    "--targets",
                    "experiments/p0/probe/src/lib.rs",
                ]
            )

        self.assertEqual(returncode, 2)
        run.assert_called_once_with(
            list_command,
            cwd=self.root,
            check=False,
            capture_output=True,
            text=True,
        )

    def test_main_requires_mutants_from_every_explicit_module(self):
        targets = (
            "experiments/p0/probe/src/lib.rs,"
            "experiments/p0/probe/src/nested/path.rs"
        )
        expected = self.command("modules", targets)
        list_command = expected[: expected.index("--")] + ["--list", "--json"]
        listed = subprocess.CompletedProcess(
            list_command,
            0,
            json.dumps(
                [
                    {
                        "file": "experiments/p0/probe/src/lib.rs",
                        "package": "thinws-p0-probe",
                    }
                ]
            ),
            "",
        )
        with (
            mock.patch.object(ci_mutation, "REPO_ROOT", self.root),
            mock.patch.object(
                ci_mutation,
                "load_workspace_package_roots",
                return_value=self.packages,
            ),
            mock.patch.object(
                ci_mutation.subprocess, "run", return_value=listed
            ) as run,
        ):
            returncode = ci_mutation.main(
                ["--scope", "modules", "--targets", targets]
            )

        self.assertEqual(returncode, 2)
        run.assert_called_once()


if __name__ == "__main__":
    unittest.main()
