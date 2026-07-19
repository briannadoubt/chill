# ADR-0006: SDK performance and privacy budgets

- Status: Accepted
- Date: 2026-07-14
- Scope ticket: CHILL-8
- Machine-readable catalog: `budgets/sdk/v1/budgets.json`
- Reference evaluator: `contracts/budgets/v1/reference.py`

## Context

Chill sits on launch paths, native command dispatch, navigation, async function
entry/exit, networking, persistent queues, and every rendered replay frame. A
small regression at each of those points becomes a large product tax. “Low
overhead” and “privacy safe” are not testable requirements, and averages from a
developer laptop do not represent a floor device under storage, radio, thermal,
or animation pressure.

The initial team and deployment budget are tiny, so the benchmark program must
be runnable by one developer without a permanent device lab. It must still be
strict enough that Chill can credibly compete with mature products. Swift is
the first implementation, while Android, web, and server SDKs must inherit the
same release discipline. React Native remains deferred and Flutter remains out
of scope.

Privacy is also a performance property. If an SDK captures arbitrary text,
screenshots, HTTP bodies, error messages, or oversized values and attempts to
redact them later, it has already spent memory, CPU, disk, and network on data
it should never have observed. Source-level default denial is both safer and
faster.

## Decision

### Budgets are release gates

Every supported feature set has a versioned profile:

- `apple.core` and `apple.replay`;
- `android.core` and `android.replay`;
- `web.core` and `web.replay`; and
- `server.core`.

`apple` initially means the iOS/iPadOS SwiftUI and UIKit vertical slice. macOS,
watchOS, tvOS, and visionOS do not silently inherit an iPhone measurement; each
needs an explicit child profile and floor before it is claimed as supported.
Android and web profiles reserve the contract for their later native SDKs.

Replay profiles inherit every core gate and add replay-specific gates. A new
platform or optional module cannot ship until it has a profile, method for each
required dimension, and a complete passing report. Every V1 budget is explicitly
release blocking. Missing data fails; it is not treated as zero or “not
applicable.”

The checked catalog is normative for exact units, statistics, minimum sample
counts, methods, and thresholds. The tables below are the human-readable
summary.

### Swift-first core limits

| Dimension | Apple core threshold |
| --- | ---: |
| App Store download-size delta | 2 MiB maximum across thinned variants |
| Installed-size delta | 5 MiB maximum across thinned variants |
| Cold-launch p95 paired delta | 10 ms upper confidence bound |
| Longest launch blocking-slice p99 delta | 2 ms upper confidence bound |
| Disabled declaration hot path | 100 ns p99, zero allocations |
| Enabled synchronous capture | 250 µs p99 per fact |
| Main-thread file/database operations | zero |
| Steady-state physical-memory delta | 4 MiB upper confidence bound |
| Typical-workload CPU delta | 2 percentage points |
| Typical-workload energy delta | 3 percent |
| Compressed batched behavior payload | 750 bytes per record |
| Idle exporter requests | zero per ten minutes |
| Core offline queue hard cap | 64 MiB |
| Logical bytes written | 1,024 bytes per record |
| High-value drops at rated load | zero |
| SDK-caused crash, hang, or ANR | zero |

The macro implementation and SwiftSyntax plugin are build-time dependencies;
they are not linked into the application runtime. Package measurements include
everything actually added to a stripped, optimized application archive,
including OpenTelemetry integrations and resources.

Equivalent core headline thresholds are:

| Platform | Download/bundle delta | Installed delta | Launch/init p95 delta | Capture p99 | Steady memory | CPU |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Apple | 2 MiB | 5 MiB | 10 ms | 250 µs | 4 MiB | 2% |
| Android | 1.5 MiB | 4 MiB | 15 ms | 250 µs | 4 MiB | 2% |
| Web | 35 KiB Brotli | n/a | 10 ms | 300 µs | 2 MiB | 2% |
| Server | 10 MiB artifact/image | n/a | 10 ms | 100 µs | 16 MiB RSS | 3% CPU and throughput |

These are deltas caused by Chill, not absolute host-application performance
targets. Host teams should still meet platform user-experience targets. For
example, Android's current guidance aims for cold starts below 500 ms, but
Chill is allowed to consume only the much smaller delta above.

### Replay limits

Replay is a separately linked and separately consented module. The default
configuration is off.

| Dimension | Apple / Android | Web |
| --- | ---: | ---: |
| Additional compressed module size | 1 MiB | 45 KiB Brotli |
| Capture and source masking | 1 ms p95 / 2 ms p99 per frame | same |
| Jank-rate delta | 0.5 percentage points | same |
| Working-memory delta | 16 MiB | 12 MiB |
| CPU delta during replay journey | 8% | 8% |
| Energy delta | 8% | measured as CPU/task impact until portable browser energy data exists |
| Compressed network rate | 150 KiB/minute | 150 KiB/minute |
| Replay offline hard cap | 256 MiB | 64 MiB |
| Replay frames suppressed by overload | 1% maximum |
| Unredacted frames or serialized canary leaks | zero | zero |
| Raw sensitive bytes in a replay buffer | zero | zero |

Replay may reduce fidelity adaptively before it harms interaction: coalesce
geometry deltas, lower snapshot cadence, suppress offscreen subtrees, then drop
low-value frames. It may not defer masking, capture raw text, or exceed its hard
memory/disk/network cap to preserve fidelity.

### Benchmark builds and baselines

Every performance result compares two applications or services produced from
the same source revision and toolchain:

1. the control compiles Chill and the tested feature out of the runtime path;
2. the candidate enables the exact release profile;
3. both use optimized release settings, stripping, minification, and the same
   endpoint, content, accounts, storage state, and network policy; and
4. runs are interleaved in randomized order on the same machine/device.

A previous SDK release is useful trend data but is not the control because host
application and toolchain changes would be confounded. Debugger-attached,
assert-enabled, simulator/emulator, or development-server numbers cannot satisfy
a method that requires a physical release measurement.

The performance floor is the slowest physical device and oldest OS/browser
combination the SDK claims to support. One current representative device is
also run to catch architecture-specific regressions. Exact models live in the
release report because minimum supported hardware will change without changing
the budget semantics.

Apple power comparisons use the same device model because Power Profiler values
are not comparable across models. Android power gates use a supported Pixel
power-rail device; the performance-floor device still runs all non-power gates.
Web uses a pinned production browser with a versioned floor CPU/network profile
and supplements it with cross-browser conformance. Server tests use an isolated
pinned runner with fixed CPU, memory, kernel, container, and collector versions.

### Statistics and noise

Measurements use paired deltas, not ratios of unrelated dashboards.

- package/configuration limits use one clean reproducible pair;
- cold launch uses at least 30 pairs;
- microbenchmarks use at least 20 independent batches, with enough iterations
  per batch that timer resolution is not material;
- sustained CPU, memory, disk, replay, energy, and reliability use at least five
  paired 30-minute trials; and
- batched network size uses at least ten 100-record batches.

For p95/p99 and mean delta gates, the reported value is the upper bound of a
95% confidence interval over paired results. A run with thermal throttling,
charging, background-update interference, unexpected radio changes, or a
missing trace is invalid and may be repeated. A valid failure may not be
repeated until a lucky pass appears. Raw samples, environment metadata, traces,
and the aggregation tool version remain attached to the report.

Daily MetricKit, Android vitals, browser field data, and server telemetry are
post-release guardrails. They can catch hardware and workload coverage gaps,
but they do not replace a pre-release report.

### Standard workloads

Every platform implements semantically equivalent workloads:

1. **Disabled hot path:** one million declared action/event/activity checks with
   collection disabled; no IDs, snapshots, allocations, locks, I/O, task hops,
   or exporter work may occur.
2. **Capture microbenchmark:** a fact with session, three-segment page path,
   active trace, eight mixed scalar/array annotations, privacy state, and
   resource reference is snapshotted and enqueued to an already-open in-memory
   queue.
3. **Typical interaction:** a 30-minute deterministic journey averaging ten
   canonical facts per second with navigation, actions, activities,
   impressions, network spans, background/foreground, and periodic export.
4. **Rated load:** 100 facts per second for 30 minutes plus 1,000 facts per
   second for ten seconds, followed by flush, process restart, and recovery.
5. **Offline pressure:** begin with a queue at 80% of its hard cap, remain
   offline through rated bursts, terminate at every commit boundary, then
   recover over a lossy connection.
6. **Replay journey:** ten minutes of scrolling recycled content, 60/120 Hz
   animation where available, navigation stacks, tabs, sheets/dialogs, forms,
   images/media placeholders, layout changes, backgrounding, and reconnect.
7. **Privacy corpus:** unique non-dictionary canaries are placed in every denied
   source and searched across live memory snapshots, persistent storage,
   exported OTLP, replay blobs, SDK logs, crash diagnostics, and server reject
   paths.

The host test application and random seed are versioned fixtures. Platform
adapters may add workloads, but they may not remove semantic cases or lower the
rated load.

### Drop and backpressure policy

At rated load, session/journey/page lifecycle, actions, declared events,
activity starts/ends, consent transitions, and crash facts have a drop budget
of zero. The exporter performs no blocking network or disk I/O on a UI thread.

Beyond rated load or at a hard storage cap, degradation is deterministic:

1. coalesce redundant replay geometry and performance samples;
2. suppress new replay frames and repeated impressions;
3. preserve lifecycle, action, event, and activity boundaries as long as any
   writable quota remains; and
4. increment bounded per-reason counters and emit one recovery diagnostic when
   export resumes.

The SDK never spins, blocks application work waiting for telemetry capacity,
or hides a drop. Sending again after an at-least-once transport failure retains
the original record ID and is not counted as a new fact.

## Privacy capture policy

### An allowlist, not a redaction wish list

Automatic capture may observe only bounded structural data:

- developer-declared semantic names and instrumentation element IDs;
- native role, state, input origin, visibility, focus, and coarse geometry;
- structured page segments and SDK-generated lifecycle IDs;
- monotonic/wall time, duration, sizes, status code/class, and route templates;
- explicitly registered booleans, bounded numbers, public enums, and opaque
  identifiers; and
- already-redacted replay structure.

The following are denied by default and are not made safe merely by hashing:

- UI text, attributed text, form values, selection, keyboard composition,
  accessibility label/value/hint, tooltips, and clipboard content;
- secure/password controls under every configuration;
- image pixels, screenshots, video, canvas/WebGL output, maps, PDFs, and custom
  drawing unless a future separately reviewed source masker proves zero raw
  buffering;
- DOM `textContent`, `innerHTML`, input values, arbitrary `data-*` attributes,
  and serialized component props/state;
- full URLs, query strings, fragments, HTTP headers/cookies, request/response
  bodies, GraphQL variables, and message/job payloads;
- file paths, database statements with values, error descriptions, exception
  associated values, and arbitrary stack-local values; and
- advertising IDs, device fingerprints, authorization credentials, secrets,
  and enumerable PII disguised as a digest.

The canonical schema's 4 KiB scalar bound is a transport hard stop, not capture
permission. V1 analytics accepts no arbitrary free-form user text even through
an explicit macro capture. A project schema must classify an allowed bounded
value, and unknown classifications fail closed at compile time when possible
and at runtime otherwise.

Network auto-instrumentation may retain method, normalized first-party route
template, protocol, status, timing, and byte counts. Dynamic URL segments,
queries, headers, and bodies never become span or behavior attributes. Error
instrumentation uses a declared stable reason/type code, not a localized or
server-provided message.

### Replay masking occurs before buffering

Structural replay stores roles, declared IDs, geometry, visibility, and bounded
state. Text nodes become fixed masks with length buckets; the original string
is never copied into a replay object. Sensitive controls and arbitrary rendered
content become masked rectangles at the source. A capture adapter cannot put an
unmasked frame in an in-memory queue “temporarily” and redact it during export:
the raw-sensitive-buffer budget is exactly zero.

Custom rendering is masked wholesale until a platform-specific safe adapter is
explicitly registered. A registration can unmask structural regions, not grant
blanket access to pixels or text.

### Consent is a hot-path gate

`essential`, `analytics`, `diagnostic`, and `replay` capture classes are
independent policy decisions. Replay never turns on merely because analytics
is allowed. When a class is denied, the runtime branches before generating a
record ID, resolving annotations, copying values, or opening storage/network
work.

Revocation immediately prevents new persistence and outbound work, cancels
pending uploads for that class, clears in-memory buffers, and makes queued data
unavailable for upload. Purge completes within 5 seconds at p95 in the standard
corpus. Bytes persisted or sent after a denied decision are both exact-zero
release gates. Essential diagnostics are limited to content-free counters and
cannot be used to bypass a denied analytics/replay class.

Encryption, OS data protection, transport security, retention limits, and
tenant authorization remain mandatory defense in depth. They do not relax any
capture or zero-leak budget.

## Platform methods

| Platform | Package | Launch / UI | CPU, memory, disk | Power | Network / field guard |
| --- | --- | --- | --- | --- | --- |
| Apple | Xcode app-thinning size report | XCTest launch, hitch, clock, and signpost metrics on devices | XCTest, Instruments, MetricKit-compatible counters | paired Power Profiler traces | Instruments/controlled endpoint; MetricKit daily reports |
| Android | optimized AAB/APK size variants | Macrobenchmark StartupTimingMetric, FrameTimingMetric, trace sections | Microbenchmark, Macrobenchmark/Perfetto, process/disk counters | Macrobenchmark PowerMetric on supported Pixel hardware | controlled endpoint; field vitals |
| Web | minified tree-shaken Brotli ESM | Lighthouse A/B and user timing | PerformanceObserver event/long-task/layout data, browser process/heap/storage counters | CPU/task proxy until portable energy data exists | browser network instrumentation and field Web Vitals |
| Server | stripped artifact/container layer | pinned process startup A/B | isolated throughput, CPU, RSS, disk, queue, and stall trials | host CPU/energy telemetry where available | local fixed collector plus production OTel guardrails |

Apple's app-thinning report is used because a raw `.app`, archive, or uploaded
IPA is not an accurate download/install measurement. Android Macrobenchmark is
run on physical hardware; official guidance explicitly discourages emulator
performance numbers. Its frame metrics expose p50/p90/p95/p99 distributions,
and its PowerMetric is system-wide and hardware-limited, which is why Chill
uses paired isolated trials. Web long tasks are observed as tasks over 50 ms;
Chill's own synchronous and initialization budgets are intentionally much
smaller.

## Report and evaluator contract

A budget report identifies the exact catalog version, profile, candidate,
compiled-out baseline, and one measurement for every resolved budget. Each
measurement states:

- metric ID, numeric value, unit, and exact statistic;
- sample count and normative method ID;
- optimized release build mode; and
- whether a physical device produced it.

The reference evaluator rejects version, profile, method, unit, statistic,
sample, build-mode, and device-provenance mismatches. Unknown and duplicate
measurements are errors. Missing measurements and threshold breaches produce a
failed release evaluation. Replay inheritance is resolved by the evaluator, so
a replay report cannot omit core budgets.

CI invokes the checked entry point and preserves its JSON result:

```sh
python3 scripts/evaluate-sdk-budgets.py path/to/benchmark-report.json
```

Exit zero passes, exit one is a valid release-blocking budget failure, and exit
two means the catalog or report is malformed and therefore cannot authorize a
release.

Budget changes require a dedicated review and an ADR/catalog version change.
A feature regression and a relaxed threshold cannot be approved in the same
change. Tightening within measurement noise first requires a calibrated method
update; otherwise the gate would encourage meaningless reruns.

## Required invariants

1. **Compiled-out baseline:** every delta compares the same host revision with
   the tested feature absent from the runtime path.
2. **Release provenance:** debug, simulator, emulator, and under-sampled data
   cannot satisfy a physical release method.
3. **Floor-device gate:** performance passes on the slowest supported client
   class, not only a current flagship.
4. **Conservative statistic:** p95/p99/mean delta gates use the upper 95%
   confidence bound.
5. **No silent missing data:** every resolved metric is present or release
   fails.
6. **Zero disabled allocation:** disabled declarations allocate nothing and
   perform no I/O, locking, task hop, or ID creation.
7. **No UI-thread I/O:** capture and enqueue paths never perform file,
   database, or network I/O on the UI thread.
8. **Bounded queues:** memory, disk, network, and replay have hard caps and
   deterministic degradation.
9. **Zero high-value drops:** rated steady/burst/offline workloads lose no
   high-value canonical fact.
10. **Default-deny content:** arbitrary UI, network, error, file, payload, and
    rendered content is never automatically captured.
11. **Zero raw replay content:** masking precedes allocation into a replay
    buffer.
12. **Independent consent:** analytics cannot implicitly enable replay or
    diagnostics.
13. **Denied means zero:** no denied-class persistence or outbound byte is
    permitted.
14. **Fail-closed classification:** unknown or invalid capture classification
    has zero accepted paths.
15. **No lucky reruns:** valid failures require a product/code change, not
    repeated sampling.

## Consequences

- The Swift runtime architecture must be lazy, allocation-free when disabled,
  and independent of the main thread for persistence/export.
- Replay fidelity is adaptive and subordinate to interaction, resource, and
  privacy budgets.
- Maintaining paired fixture apps and a small physical-device floor is real
  engineering work, but it is far cheaper than discovering overhead or leaks
  after SDK adoption.
- Some energy gates require a second supported measurement device because the
  performance floor may not expose power rails.
- Initial thresholds are intentionally demanding. If implementation evidence
  shows a method is invalid, the method is repaired transparently; the result
  is not hidden by silently widening the limit.

## Primary references

- Apple, [XCTMetric](https://developer.apple.com/documentation/XCTest/XCTMetric)
- Apple, [Monitoring app performance with MetricKit](https://developer.apple.com/documentation/metrickit/monitoring-app-performance-with-metrickit)
- Apple, [Reducing your app's size](https://developer.apple.com/documentation/xcode/reducing-your-app-s-size)
- Apple, [Measuring your app's power use with Power Profiler](https://developer.apple.com/documentation/xcode/measuring-your-app-s-power-use-with-power-profiler)
- Android Developers, [Benchmark your app](https://developer.android.com/topic/performance/benchmarking/benchmarking-overview)
- Android Developers, [Write a Macrobenchmark](https://developer.android.com/topic/performance/benchmarking/macrobenchmark-overview)
- Android Developers, [Capture Macrobenchmark metrics](https://developer.android.com/topic/performance/benchmarking/macrobenchmark-metrics)
- web.dev, [Use Lighthouse for performance budgets](https://web.dev/articles/use-lighthouse-for-performance-budgets)
- W3C, [Long Tasks API](https://w3c.github.io/longtasks/)
- MDN, [PerformanceObserver.observe](https://developer.mozilla.org/en-US/docs/Web/API/PerformanceObserver/observe)
- web.dev, [Core Web Vitals thresholds](https://web.dev/articles/defining-core-web-vitals-thresholds)
