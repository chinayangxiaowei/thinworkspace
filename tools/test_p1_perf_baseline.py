"""Focused tests for the reproducible Phase 1 local performance fixture."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from tools.p1_perf_baseline import make_source, summarize_ms


class Phase1PerfBaselineTest(unittest.TestCase):
    def test_fixture_is_deterministic_and_counts_all_files(self) -> None:
        with tempfile.TemporaryDirectory() as left_dir, tempfile.TemporaryDirectory() as right_dir:
            left = make_source(Path(left_dir), small_count=2, large_count=1, large_bytes=1024)
            right = make_source(Path(right_dir), small_count=2, large_count=1, large_bytes=1024)
            self.assertEqual(left, right)
            self.assertEqual(left["file_count"], 3)
            self.assertEqual(left["logical_bytes"], 2 * 4096 + 1024)

    def test_summary_uses_observed_nearest_rank_and_keeps_extremes(self) -> None:
        summary = summarize_ms([10.0, 2.0, 7.0, 5.0])
        self.assertEqual(summary["count"], 4)
        self.assertEqual(summary["min_ms"], 2.0)
        self.assertEqual(summary["median_ms"], 6.0)
        self.assertEqual(summary["p95_ms"], 10.0)
        self.assertEqual(summary["max_ms"], 10.0)


if __name__ == "__main__":
    unittest.main()
