from __future__ import annotations

from itertools import product
import unittest

from contracts.annotations.v1 import AnnotationDeclaration as Declaration
from contracts.annotations.v1 import AnnotationScope as Scope
from contracts.annotations.v1 import resolve_annotations


def scope(scope_id: str, depth: int, *pairs: tuple[str, object]) -> Scope:
    return Scope(
        scope_id=scope_id,
        depth=depth,
        declarations=tuple(Declaration(key, value) for key, value in pairs),
    )


class OuterFirstExamples(unittest.TestCase):
    def test_outer_value_wins(self) -> None:
        result = resolve_annotations(
            (
                scope("root", 0, ("id", "parent")),
                scope("child", 1, ("id", "child"), ("cat.kind", "tabby")),
            )
        )

        self.assertEqual(result.values, {"id": "parent", "cat.kind": "tabby"})
        self.assertEqual(result.origins["id"].scope_id, "root")
        self.assertEqual(len(result.collisions), 1)
        self.assertFalse(result.collisions[0].identical)

    def test_redundancy_is_type_sensitive(self) -> None:
        same = resolve_annotations(
            (
                scope("root", 0, ("flag", False)),
                scope("child", 1, ("flag", False)),
            )
        )
        different_type = resolve_annotations(
            (
                scope("root", 0, ("flag", False)),
                scope("child", 1, ("flag", 0)),
            )
        )

        self.assertTrue(same.collisions[0].identical)
        self.assertFalse(different_type.collisions[0].identical)

    def test_swiftui_outer_modifier_wins_when_adapter_orders_wrappers(self) -> None:
        # A later SwiftUI modifier wraps the modifier written before it. The
        # adapter therefore presents the later modifier first in ancestry.
        result = resolve_annotations(
            (
                scope("later-outer-modifier", 0, ("cat.id", "outer")),
                scope("earlier-inner-modifier", 1, ("cat.id", "inner")),
            )
        )

        self.assertEqual(result.values["cat.id"], "outer")
        self.assertEqual(result.origins["cat.id"].scope_id, "later-outer-modifier")

    def test_siblings_are_isolated(self) -> None:
        root = scope("root", 0, ("account.id", "outer"))
        left = resolve_annotations((root, scope("left", 1, ("item.id", "left"))))
        right = resolve_annotations((root, scope("right", 1, ("item.id", "right"))))

        self.assertEqual(left.values["item.id"], "left")
        self.assertEqual(right.values["item.id"], "right")
        self.assertTrue(all(origin.scope_id != "right" for origin in left.origins.values()))

    def test_removing_winner_promotes_first_remaining_declaration(self) -> None:
        chain = (
            scope("root", 0, ("id", "root")),
            scope("page", 1, ("id", "page")),
            scope("element", 2, ("id", "element")),
        )

        self.assertEqual(resolve_annotations(chain).values["id"], "root")
        self.assertEqual(resolve_annotations(chain[1:]).values["id"], "page")

    def test_snapshot_freezes_caller_owned_array(self) -> None:
        source = ["one", "two"]
        result = resolve_annotations((scope("root", 0, ("ids", source)),))
        source.append("three")

        self.assertEqual(result.values["ids"], ("one", "two"))
        with self.assertRaises(TypeError):
            result.values["ids"] = ("changed",)  # type: ignore[index]

    def test_chain_must_be_root_to_leaf(self) -> None:
        with self.assertRaisesRegex(ValueError, "strictly increasing"):
            resolve_annotations((scope("inner", 1), scope("outer", 0)))

    def test_invalid_values_and_keys_are_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "invalid annotation key"):
            resolve_annotations((scope("root", 0, ("Invalid Key", "value")),))
        with self.assertRaisesRegex(TypeError, "homogeneous"):
            resolve_annotations((scope("root", 0, ("ids", ["one", 2])),))
        with self.assertRaisesRegex(ValueError, "finite"):
            resolve_annotations((scope("root", 0, ("ratio", float("nan"))),))

    def test_scope_identity_must_be_unique(self) -> None:
        with self.assertRaisesRegex(ValueError, "duplicate scope_id"):
            resolve_annotations((scope("same", 0), scope("same", 1)))


class OuterFirstProperties(unittest.TestCase):
    OPTIONS: tuple[tuple[tuple[str, object], ...], ...] = (
        (),
        (("a", "a0"),),
        (("a", "a1"),),
        (("b", "b0"),),
        (("b", "b1"),),
        (("a", "a0"), ("b", "b0")),
        (("b", "b1"), ("a", "a1")),
        (("a", "a0"), ("a", "a1")),
    )

    def chains(self):
        for declarations in product(self.OPTIONS, repeat=4):
            yield tuple(
                scope(f"scope-{depth}", depth, *pairs)
                for depth, pairs in enumerate(declarations)
            )

    def test_first_source_selection_and_exact_collision_accounting(self) -> None:
        for chain in self.chains():
            with self.subTest(chain=chain):
                result = resolve_annotations(chain)
                flattened = [
                    declaration
                    for current_scope in chain
                    for declaration in current_scope.declarations
                ]
                for key in {declaration.key for declaration in flattened}:
                    matching = [item for item in flattened if item.key == key]
                    self.assertEqual(result.values[key], matching[0].value)
                    collisions = [item for item in result.collisions if item.key == key]
                    self.assertEqual(len(collisions), len(matching) - 1)

    def test_appending_descendant_preserves_existing_values(self) -> None:
        descendant = scope("descendant", 4, ("a", "override"), ("new", "value"))
        for chain in self.chains():
            with self.subTest(chain=chain):
                before = resolve_annotations(chain)
                after = resolve_annotations(chain + (descendant,))
                for key, value in before.values.items():
                    self.assertEqual(after.values[key], value)
                    self.assertEqual(after.origins[key], before.origins[key])

    def test_resolution_is_deterministic(self) -> None:
        for chain in self.chains():
            with self.subTest(chain=chain):
                first = resolve_annotations(chain)
                second = resolve_annotations(chain)
                self.assertEqual(first.values, second.values)
                self.assertEqual(first.origins, second.origins)
                self.assertEqual(first.collisions, second.collisions)


if __name__ == "__main__":
    unittest.main()
