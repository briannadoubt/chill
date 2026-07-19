# Contributing to Chill

Thank you for helping improve Chill. The project welcomes focused bug fixes,
documentation, tests, privacy improvements, performance work, and carefully
scoped features.

By submitting a contribution, you agree that it is licensed under the Apache
License 2.0 and that you have the right to contribute it.

## Before you start

1. Read `README.md`, `GOVERNANCE.md`, and `CODE_OF_CONDUCT.md`.
2. Search existing issues and discussions.
3. Open a discussion before large API, schema, architecture, or product-scope
   changes. Security reports belong in the private channel in `SECURITY.md`.
4. Keep pull requests small enough to review and explain the user-visible
   behavior, privacy impact, compatibility impact, and verification evidence.

## Architecture rules

- Server-side Chill components are Rust. Client SDKs stay idiomatic to their
  platforms: Swift for Apple, Kotlin for Android, and TypeScript for web.
- Application integrations declare semantic context; they do not add manual
  `track()`-style calls.
- Outer annotation scopes are authoritative. Inner scopes cannot overwrite an
  existing key.
- Text and arbitrary content are default-deny. Never add credentials, personal
  data, raw payloads, or telemetry contents to logs, fixtures, screenshots, or
  issue reports.
- Released migrations and versioned contracts are immutable. Add a new version
  instead of rewriting a released one.

## Development setup

The exact supported toolchains live in `ci/toolchains.json`. Start with the
fast repository and contract checks:

```sh
python3 -m venv .venv
.venv/bin/pip install -r requirements-contract.txt
.venv/bin/python scripts/repository_contract.py verify
.venv/bin/python scripts/validate_documentation.py
.venv/bin/python scripts/validate-contract-fixtures.py
.venv/bin/python scripts/run-conformance-suite.py
.venv/bin/python -m unittest discover -s tests -p 'test_*.py'
```

Component commands and environment-dependent integration tests are documented
in `backend/README.md`, `docs/integrating-swift.md`, `docs/integrating-web.md`,
`docs/integrating-android.md`, and `console/README.md`.

Before requesting review, run formatting, linting, unit tests, and repository
checks for every affected component. Do not weaken a gate to make a change
pass. If a hardware- or service-dependent gate cannot run, state that clearly
in the pull request and provide the strongest available local evidence.

## Pull requests

Use an imperative title and explain why the change is needed. Link its issue,
call out migrations or breaking changes, and include tests. Generated files
must be produced by their checked-in generator. Reviews may request smaller
commits or an ADR when a change crosses a durable contract boundary.
