# Governance

Chill is currently a maintainer-led open-source project. Brianna Zamora
(`@briannadoubt`) is the initial maintainer and final decision maker for
repository scope, security response, releases, and contributor access.

## Decisions

Small implementation decisions are made through issues and pull requests.
Changes to public contracts, privacy guarantees, storage formats, supported
platforms, or architecture require an ADR in `docs/architecture/` and must
preserve or explicitly version the cross-platform conformance contract.

Decisions favor user safety, privacy by default, interoperability, bounded
resource use, and maintainability. Consensus is preferred, but the maintainer
may resolve a decision when consensus is unavailable or project safety and
coherence require it.

## Maintainers

Maintainers review changes, triage reports, manage releases, and enforce the
Code of Conduct. New maintainers may be invited after sustained, high-quality
contributions and demonstrated judgment across technical and community work.
Maintainer status may be removed for inactivity, security concerns, or Code of
Conduct violations.

## Releases

The repository uses coordinated semantic versions. Only the release workflow
documented in `docs/releasing.md` publishes artifacts. Pre-1.0 APIs may change;
security and migration notes take priority over release cadence.
