from __future__ import annotations

import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools import ci_release_binary_e2e


class ReleaseBinaryE2EGuardTests(unittest.TestCase):
    def test_self_hosted_runner_is_rejected_before_product_access(self) -> None:
        with tempfile.TemporaryDirectory() as scratch:
            environment = {
                "GITHUB_ACTIONS": "true",
                "RUNNER_ENVIRONMENT": "self-hosted",
                "RUNNER_TEMP": scratch,
            }
            with (
                patch.dict(os.environ, environment, clear=True),
                patch.object(sys, "platform", "darwin"),
                patch.object(ci_release_binary_e2e, "BINARY", Path(scratch) / "absent"),
            ):
                with self.assertRaisesRegex(
                    RuntimeError, "requires a GitHub-hosted macOS runner"
                ):
                    ci_release_binary_e2e.main()


if __name__ == "__main__":
    unittest.main()
