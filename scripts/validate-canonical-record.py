#!/usr/bin/env python3
"""Validate one canonical behavior record supplied on stdin."""

from __future__ import annotations

import json
import re
import sys


def main() -> int:
    record = json.load(sys.stdin)
    errors: list[str] = []
    if record.get("kind") != "event" or record.get("operation") != "instant":
        errors.append("real Swift macro did not normalize as an instant event")
    if record.get("record_id") != record.get("subject_id"):
        errors.append("instant subject does not equal record ID")
    if record.get("payload", {}).get("emission") != "succeeded":
        errors.append("event emission was not preserved")
    if record.get("outcome", {}).get("status") != "ok":
        errors.append("successful event outcome was not normalized")
    if int(record["clock"]["observed_at_unix_nano"]) < int(
        record["clock"]["occurred_at_unix_nano"]
    ):
        errors.append("trusted observation precedes source occurrence")
    if not re.fullmatch(
        r"[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}",
        record.get("source", {}).get("installation_id", ""),
    ):
        errors.append("persistent source installation ID is not UUIDv4")
    for error in errors:
        print(f"canonical:<semantic>: {error}", file=sys.stderr)
    if errors:
        return 1
    print(f"canonical-ok:{record['record_id']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
