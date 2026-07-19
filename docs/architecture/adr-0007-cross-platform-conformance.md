# ADR-0007: Cross-platform conformance is fixture-driven and exact

- Status: Accepted
- Date: 2026-07-14
- Owners: SDK and telemetry architecture
- Decision scope: Apple, Android, web, and server SDK behavior

## Context

Chill starts with Swift, but its product contract is not “whatever the first
Swift implementation happens to do.” Automatic UI instrumentation, outer-first
annotations, structured pages, trace propagation, buffering, redaction,
sampling, and replay alignment all contain edge cases that can drift subtly
between runtimes. A prose specification alone cannot prove equivalent behavior.

Conformance must also cover the code that touches a real framework and the code
that persists or exports a fact. A platform can have a perfect pure model while
its SwiftUI observer double-counts actions or its exporter leaks a redacted
value. Unit tests for only the pure model are therefore insufficient.

## Decision

Chill V1 has a versioned manifest and deterministic JSON scenarios under
`conformance/v1`. Each scenario contains:

- a stable ID and semantic domain;
- deterministic inputs, including test-only clocks, IDs, salts, and faults;
- one exact expected JSON output;
- the layers at which the fixture must pass; and
- optional sensitive canary strings that must not occur in output.

V1 covers annotations, navigation, actions, trace propagation, offline
recovery, replay alignment, schema negotiation, redaction, and sampling.
Outputs are compared structurally and exactly. There is no field omission,
ordering normalization beyond JSON object key order, version coercion, or
platform-specific semantic exception.

The suite defines three profiles:

| Profile | Required behavior |
| --- | --- |
| `client.core` | All client semantics except replay |
| `client.replay` | `client.core` plus replay alignment |
| `server.core` | Shared server semantics without UI navigation/actions/replay |

Every applicable fixture must be executed independently at all three layers:

| Layer | What it proves |
| --- | --- |
| `model` | The SDK's pure semantic state machine matches the contract |
| `adapter` | Real framework callbacks produce the same normalized facts |
| `wire` | Persisted/recovered/exported facts retain the same semantics |

A layer is implementation provenance, not a test result selected per scenario.
An SDK claiming complete conformance publishes one report for each layer. The
report names its platform, adapter, SDK version, suite version, and suite
digest. The digest binds the manifest, inputs, expected outputs, and privacy
canaries. Missing, skipped, duplicate, unknown, or stale results are rejected.

## Determinism boundary

Production APIs do not expose clocks, ID factories, sampling salts, or storage
fault controls. A conformance adapter injects those dependencies below the
public declarative API. Tests must not wait on wall time or depend on random
identifiers, process-local hash functions, locale, dictionary order, network
availability, or device-specific text.

Platform provenance such as framework callback names may appear in a test log,
but it is not part of the expected semantic output. The suite compares what
the platform means, not which private callback delivered it.

## Pinned V1 edge behavior

- The outermost annotation declaration wins, including when an inner value is
  equal to the winner.
- A sheet extends its presenter's structured path and dismissal resumes the
  same presenter instance.
- Multiple observer callbacks sharing one native activation ID emit one action.
- HTTP trace injection requires a trusted origin; baggage is allowlisted.
- Failed exports remove nothing, acknowledgements remove only acknowledged
  records, and capacity pressure discards replay before behavioral facts.
- Replay alignment uses boot ID plus monotonic offsets and ignores wall-clock
  rollback.
- Schema versions and schema URLs negotiate as exact pairs.
- Redaction happens before buffering; privacy canaries are forbidden in every
  output layer.
- Behavior and replay use deterministic, independent SHA-256 sampling streams;
  neither inherits the OpenTelemetry trace sampled bit.

## Running the suite

Run the checked reference implementation:

```sh
python3 scripts/run-conformance-suite.py
```

Evaluate an SDK-produced report:

```sh
python3 scripts/run-conformance-suite.py path/to/sdk-report.json
```

The process exits `0` only when every required scenario passes, `1` for a
semantic mismatch, and `2` for an invalid suite or report.

## Evolution

Changing a V1 expected output is a contract change and requires an ADR plus a
new suite version. Additive scenarios may be introduced only with a new suite
version because existing SDK reports would otherwise become incomplete under
the same digest. Older suites remain available for compatibility testing.

## Consequences

- Swift can ship first without becoming an accidental cross-platform spec.
- SDK releases gain a machine-checkable semantic gate in addition to unit,
  integration, performance, and privacy tests.
- Adapter and wire harnesses require deliberate test seams in each SDK.
- Exact fixtures make intentional contract evolution more formal, which is a
  desirable cost for a multi-platform telemetry product.
