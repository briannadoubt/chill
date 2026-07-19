from __future__ import annotations

from itertools import product
import unittest

from contracts.navigation.v1 import (
    Exposure,
    NavigationState,
    PageEntry,
    Relation,
    RouteSpec,
    dismiss,
    pop,
    present,
    push,
    reconcile_linear_path,
    restore_linear_path,
    select_tab,
)


class Ids:
    def __init__(self, prefix: str = "page") -> None:
        self.prefix = prefix
        self.value = 0

    def next(self) -> str:
        self.value += 1
        return f"{self.prefix}-{self.value}"


def route(segment: str, identity: str, relation: Relation) -> RouteSpec:
    return RouteSpec(segment, identity, relation)


def root(
    instance_id: str = "root-1",
    *,
    surface_id: str = "surface-1",
    exposure: Exposure = Exposure.FOREGROUND,
    focused: bool = True,
) -> PageEntry:
    return PageEntry(
        surface_id=surface_id,
        instance_id=instance_id,
        segment="app",
        identity="app",
        parent_instance_id=None,
        relation=Relation.ROOT,
        exposure=exposure,
        focused=focused,
        order=0,
    )


class NavigationExamples(unittest.TestCase):
    def test_push_and_back_preserve_the_revealed_page_instance(self) -> None:
        state = NavigationState((root(),))
        pushed = push(
            state,
            "root-1",
            route("cat", "cat:123", Relation.PUSH),
            "cat-1",
        )

        self.assertEqual(pushed.state.primary_paths()[0].segments, ("app", "cat"))
        self.assertEqual(pushed.delta.started, ("cat-1",))
        self.assertEqual(pushed.state.by_id["root-1"].exposure, Exposure.RETAINED)

        backed = pop(pushed.state, "cat-1")
        self.assertEqual(backed.delta.ended, ("cat-1",))
        self.assertEqual(backed.state.primary_paths()[0].instance_id, "root-1")
        self.assertTrue(backed.state.by_id["root-1"].focused)

    def test_sheet_is_a_branch_and_dismissal_does_not_restart_presenter(self) -> None:
        state = NavigationState((root(),))
        shown = present(
            state,
            "root-1",
            route("settings", "settings", Relation.SHEET),
            "sheet-1",
        )

        exposed = {path.instance_id for path in shown.state.exposed_paths()}
        self.assertEqual(exposed, {"root-1", "sheet-1"})
        self.assertEqual(shown.state.primary_paths()[0].instance_id, "sheet-1")
        self.assertEqual(shown.state.by_id["root-1"].exposure, Exposure.VISIBLE)

        hidden = dismiss(shown.state, "sheet-1")
        self.assertEqual(hidden.delta.ended, ("sheet-1",))
        self.assertEqual(hidden.delta.started, ())
        self.assertEqual(hidden.state.primary_paths()[0].instance_id, "root-1")

    def test_cover_fully_occludes_presenter(self) -> None:
        shown = present(
            NavigationState((root(),)),
            "root-1",
            route("onboarding", "onboarding", Relation.COVER),
            "cover-1",
        )

        self.assertEqual(shown.state.by_id["root-1"].exposure, Exposure.OCCLUDED)
        self.assertEqual(
            tuple(path.instance_id for path in shown.state.exposed_paths()),
            ("cover-1",),
        )

    def test_nested_presentations_end_inner_first(self) -> None:
        sheet = present(
            NavigationState((root(),)),
            "root-1",
            route("settings", "settings", Relation.SHEET),
            "sheet-1",
        )
        popover = present(
            sheet.state,
            "sheet-1",
            route("help", "help", Relation.POPOVER),
            "popover-1",
        )

        self.assertEqual(
            {path.instance_id for path in popover.state.exposed_paths()},
            {"root-1", "sheet-1", "popover-1"},
        )
        dismissed = dismiss(popover.state, "sheet-1")
        self.assertEqual(dismissed.delta.ended, ("popover-1", "sheet-1"))
        self.assertEqual(dismissed.state.primary_paths()[0].instance_id, "root-1")

    def test_tabs_keep_inactive_history_and_instance_identity(self) -> None:
        state = NavigationState(
            (
                root(exposure=Exposure.VISIBLE, focused=False),
                PageEntry(
                    "surface-1",
                    "cats-tab",
                    "cats",
                    "tab:cats",
                    "root-1",
                    Relation.TAB,
                    Exposure.RETAINED,
                    False,
                    1,
                ),
                PageEntry(
                    "surface-1",
                    "cat-detail",
                    "cat",
                    "cat:123",
                    "cats-tab",
                    Relation.PUSH,
                    Exposure.RETAINED,
                    False,
                    2,
                ),
                PageEntry(
                    "surface-1",
                    "settings-tab",
                    "settings",
                    "tab:settings",
                    "root-1",
                    Relation.TAB,
                    Exposure.FOREGROUND,
                    True,
                    3,
                ),
            )
        )

        selected = select_tab(state, "cats-tab")
        self.assertEqual(selected.state.primary_paths()[0].instance_id, "cat-detail")
        self.assertEqual(
            selected.state.primary_paths()[0].segments,
            ("app", "cats", "cat"),
        )
        self.assertEqual(
            selected.state.by_id["settings-tab"].exposure, Exposure.RETAINED
        )
        self.assertEqual(selected.delta.started, ())
        self.assertEqual(selected.delta.ended, ())

    def test_split_view_can_have_several_exposed_structured_paths(self) -> None:
        state = NavigationState(
            (
                root(exposure=Exposure.VISIBLE, focused=False),
                PageEntry(
                    "surface-1",
                    "sidebar-1",
                    "cats",
                    "split:sidebar",
                    "root-1",
                    Relation.SPLIT,
                    Exposure.FOREGROUND,
                    False,
                    1,
                ),
                PageEntry(
                    "surface-1",
                    "detail-1",
                    "cat",
                    "cat:123",
                    "root-1",
                    Relation.SPLIT,
                    Exposure.FOREGROUND,
                    True,
                    2,
                ),
            )
        )

        paths = {path.instance_id: path for path in state.exposed_paths()}
        self.assertEqual(paths["sidebar-1"].segments, ("app", "cats"))
        self.assertEqual(paths["detail-1"].segments, ("app", "cat"))
        self.assertEqual(state.primary_paths()[0].instance_id, "detail-1")

    def test_each_window_has_an_independent_primary_path(self) -> None:
        state = NavigationState(
            (
                root("one-root", surface_id="window-one"),
                root("two-root", surface_id="window-two"),
            )
        )

        self.assertEqual(
            tuple(path.surface_id for path in state.primary_paths()),
            ("window-one", "window-two"),
        )

    def test_live_deep_link_preserves_only_the_common_route_prefix(self) -> None:
        ids = Ids()
        initial = restore_linear_path(
            "surface-1",
            (
                route("app", "app", Relation.ROOT),
                route("cats", "cats", Relation.PUSH),
                route("cat", "cat:123", Relation.PUSH),
            ),
            ids.next,
        ).state
        old = tuple(entry.instance_id for entry in initial.entries)

        linked = reconcile_linear_path(
            initial,
            "surface-1",
            (
                route("app", "app", Relation.ROOT),
                route("cats", "cats", Relation.PUSH),
                route("cat", "cat:999", Relation.PUSH),
            ),
            ids.next,
        )
        new = tuple(entry.instance_id for entry in linked.state.entries)

        self.assertEqual(new[:2], old[:2])
        self.assertNotEqual(new[2], old[2])
        self.assertEqual(linked.delta.ended, (old[2],))
        self.assertEqual(linked.delta.started, (new[2],))

    def test_restoration_never_reuses_persisted_page_instance_ids(self) -> None:
        snapshot = (
            route("app", "app", Relation.ROOT),
            route("cat", "cat:123", Relation.PUSH),
        )
        first = restore_linear_path("surface-1", snapshot, Ids("first").next)
        second = restore_linear_path("surface-1", snapshot, Ids("second").next)

        self.assertEqual(
            first.state.primary_paths()[0].segments,
            second.state.primary_paths()[0].segments,
        )
        self.assertTrue(
            set(first.state.by_id).isdisjoint(set(second.state.by_id))
        )

    def test_dynamic_values_do_not_enter_semantic_segments(self) -> None:
        state = restore_linear_path(
            "surface-1",
            (
                route("app", "app", Relation.ROOT),
                route("cat", "cat:12345", Relation.PUSH),
            ),
            Ids().next,
        ).state

        path = state.primary_paths()[0]
        self.assertEqual(path.segments, ("app", "cat"))
        self.assertNotIn("12345", path.segments)


class NavigationProperties(unittest.TestCase):
    def test_reconciliation_preserves_exactly_the_live_common_prefix(self) -> None:
        identities = ("a", "b")
        for old_tail, new_tail in product(identities, repeat=2):
            with self.subTest(old_tail=old_tail, new_tail=new_tail):
                ids = Ids()
                old_specs = (
                    route("app", "app", Relation.ROOT),
                    route("item", old_tail, Relation.PUSH),
                )
                new_specs = (
                    route("app", "app", Relation.ROOT),
                    route("item", new_tail, Relation.PUSH),
                )
                old = restore_linear_path(
                    "surface-1", old_specs, ids.next
                ).state
                transition = reconcile_linear_path(
                    old, "surface-1", new_specs, ids.next
                )

                old_ids = tuple(old.by_id)
                new_ids = tuple(transition.state.by_id)
                self.assertEqual(new_ids[0], old_ids[0])
                if old_tail == new_tail:
                    self.assertEqual(new_ids, old_ids)
                    self.assertEqual(transition.delta.started, ())
                    self.assertEqual(transition.delta.ended, ())
                else:
                    self.assertNotEqual(new_ids[1], old_ids[1])

    def test_every_exposed_path_is_a_rooted_instance_aligned_path(self) -> None:
        state = NavigationState(
            (
                root(exposure=Exposure.VISIBLE, focused=False),
                PageEntry(
                    "surface-1",
                    "left",
                    "left",
                    "left",
                    "root-1",
                    Relation.SPLIT,
                    Exposure.FOREGROUND,
                    False,
                    1,
                ),
                PageEntry(
                    "surface-1",
                    "right",
                    "right",
                    "right",
                    "root-1",
                    Relation.SPLIT,
                    Exposure.FOREGROUND,
                    True,
                    2,
                ),
            )
        )

        for path in state.exposed_paths():
            self.assertEqual(len(path.instance_ids), len(path.segments))
            self.assertEqual(path.instance_ids[0], "root-1")
            self.assertEqual(path.instance_id, path.instance_ids[-1])

    def test_invalid_graphs_are_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "duplicate page instance"):
            NavigationState((root(), root()))
        with self.assertRaisesRegex(ValueError, "more than one focused"):
            NavigationState(
                (
                    root(),
                    PageEntry(
                        "surface-1",
                        "child",
                        "child",
                        "child",
                        "root-1",
                        Relation.PUSH,
                        Exposure.FOREGROUND,
                        True,
                        1,
                    ),
                )
            )
        with self.assertRaisesRegex(ValueError, "focused page must be foreground"):
            NavigationState((root(exposure=Exposure.RETAINED),))


if __name__ == "__main__":
    unittest.main()
