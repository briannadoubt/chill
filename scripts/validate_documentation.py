#!/usr/bin/env python3
"""Validate durable documentation links and integration guide invariants."""

from __future__ import annotations

from pathlib import Path
import re
import sys
from urllib.parse import unquote


ROOT = Path(__file__).resolve().parents[1]
DOCUMENTS = (
    ROOT / "README.md",
    ROOT / "CODE_OF_CONDUCT.md",
    ROOT / "CONTRIBUTING.md",
    ROOT / "GOVERNANCE.md",
    ROOT / "SECURITY.md",
    ROOT / "SUPPORT.md",
    ROOT / "docs" / "project-status.md",
    ROOT / "docs" / "integrating-swift.md",
    ROOT / "docs" / "releasing.md",
    ROOT / "sdk" / "swift" / "Examples" / "README.md",
)
LINK = re.compile(r"(?<!!)\[[^\]]+\]\(([^)]+)\)")
SWIFT_FENCE = re.compile(r"```swift\n(.*?)```", re.DOTALL)
REQUIRED_SECTIONS = (
    "## Requirements and installation",
    "## Configure the runtime once",
    "## Declare pages, annotations, actions, and state events",
    "### Outer-first annotation resolution",
    "## Instrument activities and domain events with macros",
    "## UIKit integration",
    "## OpenTelemetry and network propagation",
    "## Consent, collection policy, and replay",
    "## Migration from imperative analytics",
    "## Debugging and troubleshooting",
)


def local_link_errors(document: Path, body: str) -> list[str]:
    errors: list[str] = []
    for target in LINK.findall(body):
        if target.startswith(("https://", "http://", "mailto:", "#")):
            continue
        path_text = unquote(target.split("#", 1)[0]).strip()
        if path_text.startswith("<") and path_text.endswith(">"):
            path_text = path_text[1:-1]
        if not path_text:
            continue
        resolved = (document.parent / path_text).resolve()
        try:
            resolved.relative_to(ROOT)
        except ValueError:
            errors.append(f"{document.relative_to(ROOT)}: link escapes repository: {target}")
            continue
        if not resolved.exists():
            errors.append(f"{document.relative_to(ROOT)}: missing link target: {target}")
    return errors


def verify() -> list[str]:
    errors: list[str] = []
    for document in DOCUMENTS:
        if not document.is_file():
            errors.append(f"missing required document: {document.relative_to(ROOT)}")
            continue
        body = document.read_text(encoding="utf-8")
        errors.extend(local_link_errors(document, body))

    guide = ROOT / "docs" / "integrating-swift.md"
    if not guide.is_file():
        return errors
    body = guide.read_text(encoding="utf-8")
    for section in REQUIRED_SECTIONS:
        if section not in body:
            errors.append(f"integration guide lacks section: {section}")
    for sample in (
        "ChillSwiftUISample/ChillSwiftUISample.swift",
        "ChillUIKitSample/ChillUIKitSample.swift",
        "ChillSampleSupport/SampleChill.swift",
    ):
        if sample not in body:
            errors.append(f"integration guide does not link compile-checked sample: {sample}")
    for block in SWIFT_FENCE.findall(body):
        if re.search(r"\b(?:Tracker|Chill)\.(?:track|record|capture)\s*\(", block):
            errors.append("integration guide contains an imperative telemetry call")
            break
    return errors


def main() -> int:
    errors = verify()
    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1
    print("documentation-ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
