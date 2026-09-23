import unittest

from tools.check_crate_dependencies import validate_dependency_graph


class CrateDependencyDirectionTests(unittest.TestCase):
    def test_documented_product_graph_is_accepted(self) -> None:
        graph = {
            "thinws-core": set(),
            "thinws-ports": {"thinws-core"},
            "thinws-application": {"thinws-core", "thinws-ports"},
            "thinws-cli": {"thinws-application"},
            "thinws-adapter-git-cli": {"thinws-core", "thinws-ports"},
            "thinws-adapter-macos": {"thinws-core", "thinws-ports"},
            "thinws-metadata-sqlite": {"thinws-core", "thinws-ports"},
        }

        self.assertEqual(validate_dependency_graph(graph), [])

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


if __name__ == "__main__":
    unittest.main()
