"""Unit tests for host benchmark artifact provenance guards."""

from __future__ import annotations

import hashlib
import tempfile
import unittest
from pathlib import Path

from host_transport_benchmark import check_artifacts


def catalog(kinds: list[str]) -> dict:
    return {
        "schema_version": 2,
        "modules": [{"kind": kind, "display_name": kind, "params": []} for kind in kinds],
    }


class ArtifactGuardTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(prefix="host-transport-guard-test-")
        self.addCleanup(self.temp_dir.cleanup)
        root = Path(self.temp_dir.name)
        self.binaries = {"baseline": root / "baseline", "candidate": root / "candidate"}
        self.binaries["baseline"].write_bytes(b"baseline executable")
        self.binaries["candidate"].write_bytes(b"candidate executable")
        self.result = {
            "binaries": {
                build: {"sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                for build, path in self.binaries.items()
            },
            "catalogs": {
                "baseline": catalog(["gain", "oscillator"]),
                "candidate": catalog(["gain", "oscillator"]),
            },
            "artifact_guard_inputs": {
                "catalog_policy": "unchanged",
                "baseline_kind_count": 2,
                "candidate_kind_count": 2,
                "added_kind": None,
            },
        }

    def test_unchanged_policy_accepts_identical_actual_catalogs(self) -> None:
        check_artifacts(self.result, self.binaries)
        self.assertTrue(self.result["artifact_guards"]["actual_catalogs_exactly_equal"])
        self.assertTrue(self.result["artifact_guards"]["same_positive_catalog_kind_counts"])

    def test_unchanged_policy_rejects_catalog_order_change(self) -> None:
        self.result["catalogs"]["candidate"] = catalog(["oscillator", "gain"])
        with self.assertRaisesRegex(RuntimeError, "actual_catalogs_exactly_equal"):
            check_artifacts(self.result, self.binaries)

    def test_unchanged_policy_rejects_stale_binary_hash(self) -> None:
        self.binaries["candidate"].write_bytes(b"replaced candidate executable")
        with self.assertRaisesRegex(RuntimeError, "binary_paths_still_match_recorded_sha256"):
            check_artifacts(self.result, self.binaries)

    def test_added_kind_policy_keeps_existing_validation(self) -> None:
        baseline = catalog(["gain", "oscillator"])
        candidate = catalog(["gain", "oscillator", "stock_mid_side"])
        candidate["modules"][1]["display_name"] = "Updated oscillator metadata"
        self.result["catalogs"] = {"baseline": baseline, "candidate": candidate}
        self.result["artifact_guard_inputs"].update({
            "catalog_policy": "added-kind",
            "baseline_kind_count": 2,
            "candidate_kind_count": 3,
            "added_kind": "stock_mid_side",
        })

        check_artifacts(self.result, self.binaries)
        self.assertTrue(self.result["artifact_guards"]["only_added_kind_is_stock_mid_side"])
        self.assertTrue(self.result["catalog_diff"]["changed_existing_modules"])


if __name__ == "__main__":
    unittest.main()
