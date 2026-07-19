"""Executable reference for Chill structured page and navigation semantics.

This is deliberately a small contract model, not an application router. Native
SDKs observe their platform's navigation state and must produce an equivalent
page graph and lifecycle delta. The model keeps route identity (which may be
high-cardinality) separate from exported semantic path segments.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from enum import StrEnum
import re
from typing import Callable, Iterable


_SEGMENT_PATTERN = re.compile(r"^[a-z][a-z0-9_]*(?:[.-][a-z0-9_]+)*$")


class Relation(StrEnum):
    ROOT = "root"
    PUSH = "push"
    TAB = "tab"
    SPLIT = "split"
    SHEET = "sheet"
    POPOVER = "popover"
    OVERLAY = "overlay"
    COVER = "cover"


class Exposure(StrEnum):
    """Logical render state; only FOREGROUND and VISIBLE expose pixels."""

    FOREGROUND = "foreground"
    VISIBLE = "visible"
    OCCLUDED = "occluded"
    RETAINED = "retained"


_PRESENTATIONS = {
    Relation.SHEET,
    Relation.POPOVER,
    Relation.OVERLAY,
    Relation.COVER,
}


@dataclass(frozen=True)
class RouteSpec:
    """A desired route entry.

    identity is an opaque, SDK-local reconciliation key. It may contain an
    entity identifier, but it is never exported as a semantic path segment.
    """

    segment: str
    identity: str
    relation: Relation


@dataclass(frozen=True)
class PageEntry:
    surface_id: str
    instance_id: str
    segment: str
    identity: str
    parent_instance_id: str | None
    relation: Relation
    exposure: Exposure
    focused: bool = False
    order: int = 0


@dataclass(frozen=True)
class PagePath:
    surface_id: str
    instance_id: str
    instance_ids: tuple[str, ...]
    segments: tuple[str, ...]
    relation: Relation
    exposure: Exposure
    focused: bool


@dataclass(frozen=True)
class NavigationDelta:
    """Ordered lifecycle changes for one atomic navigation transaction."""

    started: tuple[str, ...] = ()
    updated: tuple[str, ...] = ()
    ended: tuple[str, ...] = ()


@dataclass(frozen=True)
class Transition:
    state: "NavigationState"
    delta: NavigationDelta


@dataclass(frozen=True)
class NavigationState:
    entries: tuple[PageEntry, ...] = ()

    def __post_init__(self) -> None:
        _validate_entries(self.entries)

    @property
    def by_id(self) -> dict[str, PageEntry]:
        return {entry.instance_id: entry for entry in self.entries}

    def path_for(self, instance_id: str) -> PagePath:
        by_id = self.by_id
        try:
            leaf = by_id[instance_id]
        except KeyError as error:
            raise ValueError(f"unknown page instance: {instance_id}") from error

        ancestry: list[PageEntry] = []
        cursor: PageEntry | None = leaf
        seen: set[str] = set()
        while cursor is not None:
            if cursor.instance_id in seen:
                raise ValueError("page graph contains a cycle")
            seen.add(cursor.instance_id)
            ancestry.append(cursor)
            cursor = (
                by_id[cursor.parent_instance_id]
                if cursor.parent_instance_id is not None
                else None
            )
        ancestry.reverse()

        return PagePath(
            surface_id=leaf.surface_id,
            instance_id=leaf.instance_id,
            instance_ids=tuple(entry.instance_id for entry in ancestry),
            segments=tuple(entry.segment for entry in ancestry),
            relation=leaf.relation,
            exposure=leaf.exposure,
            focused=leaf.focused,
        )

    def exposed_paths(self) -> tuple[PagePath, ...]:
        exposed = {
            Exposure.FOREGROUND,
            Exposure.VISIBLE,
        }
        paths = [
            self.path_for(entry.instance_id)
            for entry in self.entries
            if entry.exposure in exposed
        ]
        return tuple(
            sorted(
                paths,
                key=lambda path: (
                    path.surface_id,
                    len(path.instance_ids),
                    self.by_id[path.instance_id].order,
                    path.instance_id,
                ),
            )
        )

    def primary_paths(self) -> tuple[PagePath, ...]:
        """Return at most one deterministic primary page for each surface."""

        visible_by_surface: dict[str, list[PagePath]] = {}
        for path in self.exposed_paths():
            visible_by_surface.setdefault(path.surface_id, []).append(path)

        relation_rank = {
            Relation.ROOT: 0,
            Relation.TAB: 1,
            Relation.PUSH: 2,
            Relation.SPLIT: 3,
            Relation.OVERLAY: 4,
            Relation.POPOVER: 5,
            Relation.SHEET: 6,
            Relation.COVER: 7,
        }
        result: list[PagePath] = []
        for surface_id in sorted(visible_by_surface):
            candidates = visible_by_surface[surface_id]
            focused = [candidate for candidate in candidates if candidate.focused]
            if focused:
                result.append(focused[0])
                continue
            result.append(
                max(
                    candidates,
                    key=lambda path: (
                        path.exposure is Exposure.FOREGROUND,
                        relation_rank[path.relation],
                        self.by_id[path.instance_id].order,
                        len(path.instance_ids),
                        path.instance_id,
                    ),
                )
            )
        return tuple(result)


def _validate_route_spec(spec: RouteSpec, *, root: bool) -> None:
    if not _SEGMENT_PATTERN.fullmatch(spec.segment):
        raise ValueError(f"invalid semantic page segment: {spec.segment}")
    if not spec.identity:
        raise ValueError("route identity must not be empty")
    if root != (spec.relation is Relation.ROOT):
        raise ValueError("the first route must be root and only it may be root")


def _validate_entries(entries: tuple[PageEntry, ...]) -> None:
    by_id: dict[str, PageEntry] = {}
    roots: dict[str, int] = {}
    focused: dict[str, int] = {}

    for entry in entries:
        if not entry.surface_id or not entry.instance_id or not entry.identity:
            raise ValueError("surface, page instance, and route identity must not be empty")
        if not _SEGMENT_PATTERN.fullmatch(entry.segment):
            raise ValueError(f"invalid semantic page segment: {entry.segment}")
        if entry.order < 0:
            raise ValueError("page order must not be negative")
        if entry.instance_id in by_id:
            raise ValueError(f"duplicate page instance: {entry.instance_id}")
        by_id[entry.instance_id] = entry
        if entry.parent_instance_id is None:
            if entry.relation is not Relation.ROOT:
                raise ValueError("a parentless page must have the root relation")
            roots[entry.surface_id] = roots.get(entry.surface_id, 0) + 1
        elif entry.relation is Relation.ROOT:
            raise ValueError("a child page cannot have the root relation")
        if entry.focused:
            if entry.exposure is not Exposure.FOREGROUND:
                raise ValueError("a focused page must be foreground")
            focused[entry.surface_id] = focused.get(entry.surface_id, 0) + 1

    if any(count != 1 for count in roots.values()):
        raise ValueError("each surface must have exactly one root")
    if any(count > 1 for count in focused.values()):
        raise ValueError("a surface cannot have more than one focused page")

    for entry in entries:
        if entry.parent_instance_id is None:
            continue
        parent = by_id.get(entry.parent_instance_id)
        if parent is None:
            raise ValueError(f"missing parent page: {entry.parent_instance_id}")
        if parent.surface_id != entry.surface_id:
            raise ValueError("parent and child pages must be on the same surface")

        seen = {entry.instance_id}
        cursor = parent
        while cursor.parent_instance_id is not None:
            if cursor.instance_id in seen:
                raise ValueError("page graph contains a cycle")
            seen.add(cursor.instance_id)
            cursor = by_id[cursor.parent_instance_id]


def _children(entries: Iterable[PageEntry], parent_id: str) -> tuple[PageEntry, ...]:
    return tuple(entry for entry in entries if entry.parent_instance_id == parent_id)


def _descendant_ids(state: NavigationState, instance_id: str) -> set[str]:
    result: set[str] = set()
    pending = [instance_id]
    while pending:
        current = pending.pop()
        if current in result:
            continue
        result.add(current)
        pending.extend(child.instance_id for child in _children(state.entries, current))
    return result


def _replace_entries(
    state: NavigationState,
    replacements: dict[str, PageEntry],
    *,
    remove: set[str] | None = None,
    append: tuple[PageEntry, ...] = (),
) -> NavigationState:
    removed = remove or set()
    entries = tuple(
        replacements.get(entry.instance_id, entry)
        for entry in state.entries
        if entry.instance_id not in removed
    ) + append
    return NavigationState(entries)


def _unfocus_surface(state: NavigationState, surface_id: str) -> dict[str, PageEntry]:
    return {
        entry.instance_id: replace(entry, focused=False)
        for entry in state.entries
        if entry.surface_id == surface_id and entry.focused
    }


def push(
    state: NavigationState,
    parent_instance_id: str,
    route: RouteSpec,
    instance_id: str,
) -> Transition:
    if route.relation is not Relation.PUSH:
        raise ValueError("push requires a push route")
    parent = state.by_id[parent_instance_id]
    replacements = _unfocus_surface(state, parent.surface_id)
    replacements[parent.instance_id] = replace(
        parent, exposure=Exposure.RETAINED, focused=False
    )
    child = PageEntry(
        surface_id=parent.surface_id,
        instance_id=instance_id,
        segment=route.segment,
        identity=route.identity,
        parent_instance_id=parent.instance_id,
        relation=Relation.PUSH,
        exposure=Exposure.FOREGROUND,
        focused=True,
        order=max((entry.order for entry in state.entries), default=-1) + 1,
    )
    next_state = _replace_entries(state, replacements, append=(child,))
    return Transition(
        next_state,
        NavigationDelta(started=(child.instance_id,), updated=(parent.instance_id,)),
    )


def pop(state: NavigationState, instance_id: str) -> Transition:
    entry = state.by_id[instance_id]
    if entry.relation is not Relation.PUSH or entry.parent_instance_id is None:
        raise ValueError("pop requires a pushed page")
    removed = _descendant_ids(state, instance_id)
    parent = state.by_id[entry.parent_instance_id]
    replacements = _unfocus_surface(state, entry.surface_id)
    replacements[parent.instance_id] = replace(
        parent, exposure=Exposure.FOREGROUND, focused=True
    )
    ended = tuple(
        item.instance_id
        for item in sorted(
            (state.by_id[item_id] for item_id in removed),
            key=lambda item: len(state.path_for(item.instance_id).instance_ids),
            reverse=True,
        )
    )
    next_state = _replace_entries(state, replacements, remove=removed)
    return Transition(
        next_state,
        NavigationDelta(updated=(parent.instance_id,), ended=ended),
    )


def present(
    state: NavigationState,
    presenter_instance_id: str,
    route: RouteSpec,
    instance_id: str,
) -> Transition:
    if route.relation not in _PRESENTATIONS:
        raise ValueError("present requires a presentation relation")
    presenter = state.by_id[presenter_instance_id]
    replacements = _unfocus_surface(state, presenter.surface_id)
    presenter_exposure = (
        Exposure.OCCLUDED if route.relation is Relation.COVER else Exposure.VISIBLE
    )
    replacements[presenter.instance_id] = replace(
        presenter, exposure=presenter_exposure, focused=False
    )
    child = PageEntry(
        surface_id=presenter.surface_id,
        instance_id=instance_id,
        segment=route.segment,
        identity=route.identity,
        parent_instance_id=presenter.instance_id,
        relation=route.relation,
        exposure=Exposure.FOREGROUND,
        focused=True,
        order=max((entry.order for entry in state.entries), default=-1) + 1,
    )
    next_state = _replace_entries(state, replacements, append=(child,))
    return Transition(
        next_state,
        NavigationDelta(started=(child.instance_id,), updated=(presenter.instance_id,)),
    )


def dismiss(state: NavigationState, instance_id: str) -> Transition:
    entry = state.by_id[instance_id]
    if entry.relation not in _PRESENTATIONS or entry.parent_instance_id is None:
        raise ValueError("dismiss requires a presented page")
    removed = _descendant_ids(state, instance_id)
    presenter = state.by_id[entry.parent_instance_id]
    replacements = _unfocus_surface(state, presenter.surface_id)
    replacements[presenter.instance_id] = replace(
        presenter, exposure=Exposure.FOREGROUND, focused=True
    )
    ended = tuple(
        item.instance_id
        for item in sorted(
            (state.by_id[item_id] for item_id in removed),
            key=lambda item: len(state.path_for(item.instance_id).instance_ids),
            reverse=True,
        )
    )
    next_state = _replace_entries(state, replacements, remove=removed)
    return Transition(
        next_state,
        NavigationDelta(updated=(presenter.instance_id,), ended=ended),
    )


def select_tab(state: NavigationState, tab_instance_id: str) -> Transition:
    selected = state.by_id[tab_instance_id]
    if selected.relation is not Relation.TAB or selected.parent_instance_id is None:
        raise ValueError("tab selection requires a tab root")

    sibling_tabs = tuple(
        entry
        for entry in state.entries
        if entry.parent_instance_id == selected.parent_instance_id
        and entry.relation is Relation.TAB
    )
    if not sibling_tabs:
        raise ValueError("tab group has no entries")

    replacements = _unfocus_surface(state, selected.surface_id)
    selected_ids = _descendant_ids(state, selected.instance_id)
    inactive_ids: set[str] = set()
    for sibling in sibling_tabs:
        if sibling.instance_id != selected.instance_id:
            inactive_ids.update(_descendant_ids(state, sibling.instance_id))

    for instance_id in inactive_ids:
        entry = state.by_id[instance_id]
        replacements[instance_id] = replace(
            entry, exposure=Exposure.RETAINED, focused=False
        )

    leaves = [
        state.by_id[instance_id]
        for instance_id in selected_ids
        if not (_descendant_ids(state, instance_id) - {instance_id})
    ]
    leaf = max(
        leaves,
        key=lambda entry: (
            len(state.path_for(entry.instance_id).instance_ids),
            entry.order,
            entry.instance_id,
        ),
    )
    for instance_id in selected_ids:
        entry = state.by_id[instance_id]
        replacements[instance_id] = replace(
            entry,
            exposure=(
                Exposure.FOREGROUND
                if instance_id == leaf.instance_id
                else Exposure.RETAINED
            ),
            focused=instance_id == leaf.instance_id,
        )

    changed = tuple(
        instance_id
        for instance_id, replacement in replacements.items()
        if replacement != state.by_id[instance_id]
    )
    return Transition(
        _replace_entries(state, replacements),
        NavigationDelta(updated=changed),
    )


def reconcile_linear_path(
    state: NavigationState,
    surface_id: str,
    desired: tuple[RouteSpec, ...],
    id_factory: Callable[[], str],
) -> Transition:
    """Atomically replace one root/push path, preserving its live common prefix."""

    if not desired:
        raise ValueError("a surface path must contain a root")
    for index, spec in enumerate(desired):
        _validate_route_spec(spec, root=index == 0)
        if index > 0 and spec.relation is not Relation.PUSH:
            raise ValueError("a linear path may contain only root then push entries")

    surface_entries = tuple(
        entry for entry in state.entries if entry.surface_id == surface_id
    )
    for entry in surface_entries:
        children = _children(surface_entries, entry.instance_id)
        if len(children) > 1 or any(child.relation is not Relation.PUSH for child in children):
            raise ValueError("reconciliation requires an unbranched root/push surface")

    current: tuple[PageEntry, ...] = ()
    if surface_entries:
        root = next(entry for entry in surface_entries if entry.relation is Relation.ROOT)
        ordered = [root]
        while True:
            children = _children(surface_entries, ordered[-1].instance_id)
            if not children:
                break
            ordered.append(children[0])
        current = tuple(ordered)

    common = 0
    for existing, requested in zip(current, desired):
        if (
            existing.segment,
            existing.identity,
            existing.relation,
        ) != (
            requested.segment,
            requested.identity,
            requested.relation,
        ):
            break
        common += 1

    preserved = list(current[:common])
    replacements: dict[str, PageEntry] = {}
    created: list[PageEntry] = []
    parent_id = preserved[-1].instance_id if preserved else None
    next_order = max((entry.order for entry in state.entries), default=-1) + 1
    for spec in desired[common:]:
        entry = PageEntry(
            surface_id=surface_id,
            instance_id=id_factory(),
            segment=spec.segment,
            identity=spec.identity,
            parent_instance_id=parent_id,
            relation=spec.relation,
            exposure=Exposure.RETAINED,
            focused=False,
            order=next_order,
        )
        next_order += 1
        created.append(entry)
        parent_id = entry.instance_id

    normalized = [
        replace(
            entry,
            exposure=(
                Exposure.FOREGROUND
                if index == len(preserved) + len(created) - 1
                else Exposure.RETAINED
            ),
            focused=index == len(preserved) + len(created) - 1,
        )
        for index, entry in enumerate(preserved + created)
    ]
    normalized_preserved = normalized[:common]
    created = normalized[common:]
    replacements.update(
        {
            entry.instance_id: updated
            for entry, updated in zip(
                preserved, normalized_preserved, strict=True
            )
        }
    )

    removed = {entry.instance_id for entry in current[common:]}
    ended = tuple(entry.instance_id for entry in reversed(current[common:]))
    updated_ids = tuple(
        entry.instance_id
        for entry, updated in zip(preserved, normalized_preserved, strict=True)
        if updated != entry
    )
    without_surface_suffix = tuple(
        entry
        for entry in state.entries
        if entry.surface_id != surface_id or entry.instance_id not in removed
    )
    replaced_preserved = tuple(
        replacements.get(entry.instance_id, entry) for entry in without_surface_suffix
    )
    next_state = NavigationState(replaced_preserved + tuple(created))
    return Transition(
        next_state,
        NavigationDelta(
            started=tuple(entry.instance_id for entry in created),
            updated=updated_ids,
            ended=ended,
        ),
    )


def restore_linear_path(
    surface_id: str,
    snapshot: tuple[RouteSpec, ...],
    id_factory: Callable[[], str],
) -> Transition:
    """Restore semantic route state while intentionally minting new instances."""

    return reconcile_linear_path(NavigationState(), surface_id, snapshot, id_factory)
