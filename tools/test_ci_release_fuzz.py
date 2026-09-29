from __future__ import annotations

import tempfile
import tomllib
import unittest
from os import environ
from pathlib import Path
from subprocess import CompletedProcess, TimeoutExpired
from unittest.mock import patch

from tools import ci_release_fuzz


class ReleaseFuzzTests(unittest.TestCase):
    def test_release_workflow_is_manual_and_runs_manifest_driven_entrypoint(self) -> None:
        workflow = (
            ci_release_fuzz.REPO_ROOT / ".github/workflows/p1-release-fuzz.yml"
        ).read_text(encoding="utf-8")
        self.assertIn("on:\n  workflow_dispatch:", workflow)
        self.assertNotIn("  push:", workflow)
        self.assertNotIn("  pull_request:", workflow)
        self.assertIn("run: python3 -B tools/ci_release_fuzz.py", workflow)

    def test_all_manifest_targets_are_selected_once(self) -> None:
        manifest = ci_release_fuzz.REPO_ROOT / "fuzz/Cargo.toml"
        targets = ci_release_fuzz.target_names(manifest)
        self.assertEqual(len(targets), 11)
        self.assertEqual(len(targets), len(set(targets)))
        self.assertIn("thinws_bootstrap_document", targets)
        self.assertIn("thinws_materialization_path", targets)
        self.assertIn("thinws_platform_ownership_document", targets)

    def test_platform_selection_does_not_count_macos_only_targets_on_linux(self) -> None:
        manifest = ci_release_fuzz.REPO_ROOT / "fuzz/Cargo.toml"
        all_targets = ci_release_fuzz.target_names(manifest)
        macos_targets = ci_release_fuzz.eligible_target_names(manifest, "darwin")
        linux_targets = ci_release_fuzz.eligible_target_names(manifest, "linux")
        self.assertEqual(macos_targets, all_targets)
        self.assertEqual(
            set(all_targets) - set(linux_targets),
            {
                "thinws_probe_path_validation",
                "thinws_materialize_layout_policy",
                "thinws_bootstrap_document",
            },
        )
        self.assertIn("thinws_platform_ownership_document", linux_targets)

    def test_linux_git_status_target_has_its_shared_cleanup_dependency(self) -> None:
        manifest = tomllib.loads(
            (ci_release_fuzz.REPO_ROOT / "fuzz/Cargo.toml").read_text(encoding="utf-8")
        )
        self.assertIn(
            "thinws_git_status",
            ci_release_fuzz.eligible_target_names(
                ci_release_fuzz.REPO_ROOT / "fuzz/Cargo.toml", "linux"
            ),
        )
        self.assertIn("thinws-p0-cleanup", manifest["dependencies"])

    def test_unknown_platform_or_manifest_target_is_rejected(self) -> None:
        manifest = ci_release_fuzz.REPO_ROOT / "fuzz/Cargo.toml"
        with self.assertRaisesRegex(ValueError, "unsupported fuzz platform"):
            ci_release_fuzz.eligible_target_names(manifest, "win32")
        with tempfile.TemporaryDirectory() as scratch:
            invalid = Path(scratch) / "Cargo.toml"
            invalid.write_text(
                '[package.metadata.thinws]\n'
                'macos_only_fuzz_targets = ["missing"]\n'
                '[[bin]]\nname = "present"\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "unknown macOS-only fuzz target"):
                ci_release_fuzz.eligible_target_names(invalid, "linux")

    def test_missing_or_duplicate_platform_classification_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as scratch:
            invalid = Path(scratch) / "Cargo.toml"
            invalid.write_text('[[bin]]\nname = "present"\n', encoding="utf-8")
            with self.assertRaisesRegex(
                ValueError, "missing macOS-only fuzz target classification"
            ):
                ci_release_fuzz.eligible_target_names(invalid, "linux")
            invalid.write_text(
                '[package.metadata.thinws]\n'
                'macos_only_fuzz_targets = ["present", "present"]\n'
                '[[bin]]\nname = "present"\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "duplicate macOS-only fuzz target"):
                ci_release_fuzz.eligible_target_names(invalid, "linux")

    def test_duplicate_manifest_target_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as scratch:
            manifest = Path(scratch) / "Cargo.toml"
            manifest.write_text(
                '[[bin]]\nname = "thinws_duplicate"\n'
                '[[bin]]\nname = "thinws_duplicate"\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "duplicate fuzz target"):
                ci_release_fuzz.target_names(manifest)

    def test_fixed_long_budget_and_bootstrap_input_limit(self) -> None:
        nightly = "nightly-2026-08-14"
        corpus = Path("/tmp/disposable-corpus")
        ordinary = ci_release_fuzz.fuzz_command("thinws_workspace_name", nightly, corpus)
        bootstrap = ci_release_fuzz.fuzz_command("thinws_bootstrap_document", nightly, corpus)
        self.assertIn("-max_total_time=300", ordinary)
        self.assertIn("-max_len=4096", ordinary)
        self.assertIn("-max_len=65537", bootstrap)
        self.assertIn("-verbosity=0", ordinary)
        self.assertEqual(ordinary[:4], ["cargo", "+" + nightly, "fuzz", "run"])
        self.assertEqual(ordinary[5], str(corpus))

    def test_existing_seeds_are_copied_to_disposable_corpus(self) -> None:
        observed: list[Path] = []

        def inspect_run(command: list[str], **_: object) -> CompletedProcess[str]:
            corpus = Path(command[5])
            self.assertTrue(corpus.is_dir())
            self.assertTrue((corpus / "name").is_file())
            self.assertFalse(corpus.is_relative_to(ci_release_fuzz.REPO_ROOT))
            observed.append(corpus)
            return CompletedProcess(command, 0)

        with (
            patch.dict(environ, {"THINWS_FUZZ_NIGHTLY": "nightly-test"}, clear=True),
            patch.object(
                ci_release_fuzz,
                "eligible_target_names",
                return_value=["thinws_remove_request"],
            ),
            patch.object(ci_release_fuzz.subprocess, "run", side_effect=inspect_run),
            patch.object(ci_release_fuzz, "record_result"),
        ):
            self.assertEqual(ci_release_fuzz.main(), 0)
        self.assertEqual(len(observed), 1)
        self.assertFalse(observed[0].exists())

    def test_failure_stops_without_marking_later_targets_passed(self) -> None:
        with (
            patch.dict(environ, {"THINWS_FUZZ_NIGHTLY": "nightly-test"}, clear=True),
            patch.object(
                ci_release_fuzz, "eligible_target_names", return_value=["first", "second"]
            ),
            patch.object(
                ci_release_fuzz.subprocess,
                "run",
                return_value=CompletedProcess([], 1),
            ) as run,
            patch.object(ci_release_fuzz, "record_result") as record,
        ):
            self.assertEqual(ci_release_fuzz.main(), 1)
            self.assertEqual(run.call_count, 1)
            record.assert_called_once_with("first", "failed (exit 1)")

    def test_process_timeout_stops_the_gate(self) -> None:
        with (
            patch.dict(environ, {"THINWS_FUZZ_NIGHTLY": "nightly-test"}, clear=True),
            patch.object(ci_release_fuzz, "eligible_target_names", return_value=["first"]),
            patch.object(
                ci_release_fuzz.subprocess,
                "run",
                side_effect=TimeoutExpired(["cargo"], 420),
            ),
            patch.object(ci_release_fuzz, "record_result") as record,
        ):
            self.assertEqual(ci_release_fuzz.main(), 1)
            record.assert_called_once_with("first", "process timeout")


if __name__ == "__main__":
    unittest.main()
