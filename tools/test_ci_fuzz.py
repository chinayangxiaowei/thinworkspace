from __future__ import annotations

import re
import tomllib
import unittest
from collections import Counter
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class FuzzWorkflowCoverageTests(unittest.TestCase):
    def test_every_declared_target_has_one_ci_smoke(self) -> None:
        manifest = tomllib.loads((ROOT / "fuzz/Cargo.toml").read_text(encoding="utf-8"))
        targets = Counter(binary["name"] for binary in manifest["bin"])
        workflow = (ROOT / ".github/workflows/p0.yml").read_text(encoding="utf-8")
        invoked = Counter(
            re.findall(
                r"^\s*run:\s*cargo\s+[^\n]*\bfuzz\s+run\s+(thinws_[a-z_]+)\b",
                workflow,
                flags=re.MULTILINE,
            )
        )

        self.assertEqual(invoked, targets)


if __name__ == "__main__":
    unittest.main()
