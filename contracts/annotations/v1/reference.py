"""Framework-neutral reference for Chill outer-first annotation resolution.

This implementation favors clarity over hot-path optimization. Native SDKs may
use persistent maps, arenas, interning, or generated code as long as they remain
observationally equivalent to this contract.
"""

from __future__ import annotations

from dataclasses import dataclass
import math
import re
from types import MappingProxyType
from typing import Mapping, TypeAlias


AnnotationScalar: TypeAlias = str | bool | int | float
AnnotationValue: TypeAlias = AnnotationScalar | tuple[AnnotationScalar, ...]

_KEY_PATTERN = re.compile(r"^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*$")


@dataclass(frozen=True)
class AnnotationDeclaration:
    key: str
    value: AnnotationValue | list[AnnotationScalar]


@dataclass(frozen=True)
class AnnotationScope:
    scope_id: str
    depth: int
    declarations: tuple[AnnotationDeclaration, ...]


@dataclass(frozen=True)
class AnnotationOrigin:
    scope_id: str
    depth: int
    declaration_index: int


@dataclass(frozen=True)
class Collision:
    key: str
    winner: AnnotationOrigin
    loser: AnnotationOrigin
    identical: bool


@dataclass(frozen=True)
class AnnotationResolution:
    values: Mapping[str, AnnotationValue]
    origins: Mapping[str, AnnotationOrigin]
    collisions: tuple[Collision, ...]


def _scalar_kind(value: AnnotationScalar) -> type[AnnotationScalar]:
    # Python considers bool a subtype of int. Chill does not.
    if isinstance(value, bool):
        return bool
    if isinstance(value, str):
        return str
    if isinstance(value, int):
        return int
    if isinstance(value, float):
        if not math.isfinite(value):
            raise ValueError("annotation numbers must be finite")
        return float
    raise TypeError(f"unsupported annotation scalar: {type(value).__name__}")


def _freeze_value(value: AnnotationValue | list[AnnotationScalar]) -> AnnotationValue:
    if isinstance(value, (list, tuple)):
        frozen = tuple(value)
        if not frozen:
            return frozen
        expected = _scalar_kind(frozen[0])
        if any(_scalar_kind(item) is not expected for item in frozen[1:]):
            raise TypeError("annotation arrays must be homogeneous")
        return frozen
    _scalar_kind(value)
    return value


def _same_typed_value(left: AnnotationValue, right: AnnotationValue) -> bool:
    if isinstance(left, tuple) or isinstance(right, tuple):
        if not isinstance(left, tuple) or not isinstance(right, tuple):
            return False
        if len(left) != len(right):
            return False
        return all(_same_typed_value(a, b) for a, b in zip(left, right, strict=True))
    return _scalar_kind(left) is _scalar_kind(right) and left == right


def _validate_chain(scopes: tuple[AnnotationScope, ...]) -> None:
    seen_scope_ids: set[str] = set()
    previous_depth = -1
    for scope in scopes:
        if not scope.scope_id:
            raise ValueError("scope_id must not be empty")
        if scope.scope_id in seen_scope_ids:
            raise ValueError(f"duplicate scope_id: {scope.scope_id}")
        if scope.depth <= previous_depth:
            raise ValueError("scopes must have strictly increasing outer-to-inner depth")
        seen_scope_ids.add(scope.scope_id)
        previous_depth = scope.depth


def resolve_annotations(scopes: tuple[AnnotationScope, ...]) -> AnnotationResolution:
    """Resolve one root-to-leaf scope chain with first-declaration precedence."""

    _validate_chain(scopes)
    values: dict[str, AnnotationValue] = {}
    origins: dict[str, AnnotationOrigin] = {}
    collisions: list[Collision] = []

    for scope in scopes:
        for index, declaration in enumerate(scope.declarations):
            if not _KEY_PATTERN.fullmatch(declaration.key):
                raise ValueError(f"invalid annotation key: {declaration.key}")

            value = _freeze_value(declaration.value)
            origin = AnnotationOrigin(scope.scope_id, scope.depth, index)
            winner = origins.get(declaration.key)
            if winner is None:
                values[declaration.key] = value
                origins[declaration.key] = origin
                continue

            collisions.append(
                Collision(
                    key=declaration.key,
                    winner=winner,
                    loser=origin,
                    identical=_same_typed_value(values[declaration.key], value),
                )
            )

    return AnnotationResolution(
        values=MappingProxyType(values),
        origins=MappingProxyType(origins),
        collisions=tuple(collisions),
    )
