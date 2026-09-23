import json
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.check_crate_dependencies import load_product_graph, validate_dependency_graph


class CrateDependencyDirectionTests(unittest.TestCase):
    def test_documented_product_graph_is_accepted(self) -> None:
        graph = {
            "thinws-core": {"thiserror", "uuid"},
            "thinws-ports": {"thinws-core"},
            "thinws-application": {"thinws-core", "thinws-ports"},
            "thinws-cli": {
                "clap",
                "directories",
                "serde_json",
                "tempfile",
                "thinws-adapter-macos",
                "thinws-application",
                "thinws-metadata-sqlite",
            },
            "thinws-adapter-git-cli": {"thinws-core", "thinws-ports"},
            "thinws-adapter-macos": {"blake3", "thinws-core", "thinws-ports"},
            "thinws-metadata-sqlite": {"thinws-core", "thinws-ports"},
        }

        self.assertEqual(validate_dependency_graph(graph), [])

    def test_core_accepts_only_its_reviewed_external_dependencies(self) -> None:
        self.assertEqual(
            validate_dependency_graph({"thinws-core": {"thiserror", "uuid"}}),
            [],
        )
        self.assertEqual(
            validate_dependency_graph({"thinws-core": {"rusqlite"}}),
            ["thinws-core must not depend on rusqlite"],
        )

    def test_activated_persistence_crates_have_exact_external_surfaces(self) -> None:
        self.assertEqual(
            validate_dependency_graph(
                {
                    "thinws-ports": {"thinws-core"},
                    "thinws-metadata-sqlite": {
                        "thinws-core",
                        "thinws-ports",
                        "rusqlite",
                        "tempfile",
                    },
                }
            ),
            [],
        )
        self.assertEqual(
            validate_dependency_graph({"thinws-metadata-sqlite": {"serde"}}),
            ["thinws-metadata-sqlite must not depend on serde"],
        )

    def test_macos_adapter_has_only_reviewed_platform_dependencies(self) -> None:
        self.assertEqual(
            validate_dependency_graph(
                {
                    "thinws-adapter-macos": {
                        "blake3",
                        "libc",
                        "thinws-core",
                        "thinws-ports",
                        "rustix",
                        "serde",
                        "tempfile",
                        "toml",
                    }
                }
            ),
            [],
        )
        self.assertEqual(
            validate_dependency_graph({"thinws-adapter-macos": {"rusqlite"}}),
            ["thinws-adapter-macos must not depend on rusqlite"],
        )

    def test_cli_can_wire_adapters_but_not_depend_on_core_or_sqlite_api(self) -> None:
        self.assertEqual(
            validate_dependency_graph(
                {
                    "thinws-cli": {
                        "clap",
                        "directories",
                        "serde_json",
                        "tempfile",
                        "thinws-adapter-macos",
                        "thinws-application",
                        "thinws-metadata-sqlite",
                    }
                }
            ),
            [],
        )
        self.assertEqual(
            validate_dependency_graph({"thinws-cli": {"thinws-core", "rusqlite"}}),
            [
                "thinws-cli must not depend on rusqlite",
                "thinws-cli must not depend on thinws-core",
            ],
        )

    def test_core_cannot_depend_on_an_adapter(self) -> None:
        violations = validate_dependency_graph(
            {"thinws-core": {"thinws-adapter-macos"}}
        )

        self.assertEqual(
            violations,
            ["thinws-core must not depend on thinws-adapter-macos"],
        )

    def test_adapters_cannot_depend_on_each_other(self) -> None:
        violations = validate_dependency_graph(
            {"thinws-adapter-macos": {"thinws-adapter-git-cli"}}
        )

        self.assertEqual(
            violations,
            ["thinws-adapter-macos must not depend on thinws-adapter-git-cli"],
        )

    def test_unknown_product_crate_has_no_implicit_permission(self) -> None:
        violations = validate_dependency_graph({"thinws-future-service": set()})

        self.assertEqual(
            violations,
            ["thinws-future-service has no approved dependency rule"],
        )

    @patch("tools.check_crate_dependencies.subprocess.run")
    def test_metadata_extraction_keeps_every_core_direct_dependency(
        self, run
    ) -> None:
        run.return_value.stdout = json.dumps(
            {
                "workspace_members": ["core-id"],
                "packages": [
                    {
                        "id": "core-id",
                        "name": "thinws-core",
                        "manifest_path": "/repo/crates/thinws-core/Cargo.toml",
                        "dependencies": [
                            {
                                "name": "thinws-p0-probe",
                                "path": "/repo/experiments/p0/probe",
                            },
                            {"name": "rusqlite", "path": None},
                            {
                                "name": "thinws-adapter-macos",
                                "path": "/outside/thinws-adapter-macos",
                            },
                        ],
                    }
                ],
            }
        )

        graph = load_product_graph(Path("/repo"))

        self.assertEqual(
            validate_dependency_graph(graph),
            [
                "thinws-core must not depend on rusqlite",
                "thinws-core must not depend on thinws-adapter-macos",
                "thinws-core must not depend on thinws-p0-probe",
            ],
        )


if __name__ == "__main__":
    unittest.main()
