from __future__ import annotations

import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class PlatformWorkflowEnvironmentTests(unittest.TestCase):
    def test_p1_cross_volume_tests_use_the_prepared_apfs_mount(self) -> None:
        workflow = (ROOT / ".github/workflows/p0.yml").read_text(encoding="utf-8")

        def runner_temp_suffix(variable: str) -> str:
            match = re.search(
                rf"printf '{variable}=%s([^']*)\\n' \"\$RUNNER_TEMP\" >> \"\$GITHUB_ENV\"",
                workflow,
            )
            self.assertIsNotNone(match, f"{variable} is not exported")
            return match.group(1)

        self.assertEqual(
            runner_temp_suffix("THINWS_P1_CROSS_VOLUME_ROOT"),
            runner_temp_suffix("THINWS_P0_CROSS_VOLUME_ROOT"),
        )
        self.assertIn(
            'hdiutil attach -nobrowse -owners on -mountpoint "$THINWS_P0_CROSS_VOLUME_ROOT"',
            workflow,
        )
        self.assertIn(
            "cargo test --locked --workspace --all-targets -- --include-ignored --nocapture",
            workflow,
        )
        self.assertIn(
            "cargo test --locked --release --workspace --all-targets -- --include-ignored --nocapture",
            workflow,
        )


if __name__ == "__main__":
    unittest.main()
