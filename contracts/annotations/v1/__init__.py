"""Version 1 outer-first annotation resolution contract."""

from .reference import (
    AnnotationDeclaration,
    AnnotationOrigin,
    AnnotationResolution,
    AnnotationScope,
    Collision,
    resolve_annotations,
)

__all__ = [
    "AnnotationDeclaration",
    "AnnotationOrigin",
    "AnnotationResolution",
    "AnnotationScope",
    "Collision",
    "resolve_annotations",
]
