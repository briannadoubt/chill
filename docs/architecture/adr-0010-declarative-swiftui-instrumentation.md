# ADR-0010: SwiftUI instrumentation is semantic and declarative

- Status: Accepted
- Date: 2026-07-15
- Owners: Apple SDK architecture
- Scope ticket: CHILL-13

## Context

Chill must observe SwiftUI behavior without application-authored tracking
calls, private framework hooks, arbitrary view-tree reflection, or implicit
capture of labels and state values. SwiftUI rebuilds view values frequently,
so declaration evaluation is not an action or page lifecycle boundary.
Navigation can retain offscreen entries, controls can activate through touch,
keyboard, voice, remote, or accessibility, and one application can own several
independent scenes.

Privacy restrictions and annotations are scoped values. The product contract
requires outer annotations to win collisions, while privacy can only become
more restrictive toward a descendant. Crash and performance observations use
their own consent class and must not smuggle raw system diagnostic payloads
into behavior records.

## Decision

The `ChillSwiftUI` product exposes declarative modifiers and native semantic
controls. The umbrella `Chill` product re-exports this surface.

```swift
CatView()
  .annotation("cat.id", value: cat.id)
  .event("cat.favorite.changed", when: favorite)
  .page("cat", relation: .push)
  .privacy(.redacted)
  .replay(.masked)
```

There is no public `track`, `record`, or `capture` function. Invalid dynamic
semantic names fail closed and emit nothing. Annotation values use the bounded
core value model; dictionary declarations are sorted before becoming one
atomic scope.

### Annotation and privacy scope

SwiftUI environment values carry immutable `AnnotationContext` snapshots.
Because later modifiers are structurally outer in SwiftUI, the outer modifier
adds its declaration first and the core resolver preserves it when an inner
scope declares the same key. Siblings receive independent immutable contexts.

Privacy and replay dispositions form monotone restrictions. A descendant may
change `standard` to `redacted` or `blocked`, and `automatic` to `masked` or
`blocked`; it cannot relax an ancestor. A blocked subtree creates no behavior
draft. Labels, accessibility strings, entered text, selection values, and
pixels are never captured by these adapters.

### Structured page surfaces

Every `.page` declaration owns stable UUIDv7 page and surface state for its
SwiftUI lifetime. An environment ancestry value composes nested semantic
segments and aligned page-instance IDs into a structured `PagePath`; path
strings are never flattened and parsed. A per-root `PageSurfaceCoordinator`
maintains one graph for each scene or window rather than a process-global
current page.

The coordinator derives ancestor exposure from active descendant relations:

- a push or selected tab branch retains its presenter;
- a sheet, popover, overlay, or split child leaves its presenter visible;
- a cover occludes its presenter; and
- the deepest, most recently mounted eligible page is the sole focused page.

Appearance starts a page lifetime once. Reappearance or scene transitions
update the same instance, disappearance retains it, and destruction ends it
once. Popped pages and dismissed presentations reveal the existing presenter
without restarting it. Stateful SwiftUI navigation is the precise path;
framework navigation that exposes only visual lifecycle remains best effort.

Root pages also produce bounded scene lifecycle events. The first mounted
render produces a monotonic `ui.page.first_render` diagnostic duration.

### Actions, forms, impressions, and observed state

`ChillActionButton`, `ChillToggle`, and `ChillPicker` wrap native SwiftUI
controls and observe their committed action or binding boundary. They preserve
native control semantics and never inspect the visible label or bound value.
The wrapped button operation executes inside the current annotation and page
task-local context, so `@Activity` and `@Event` macros inherit the UI context.

The generic `.action` modifier supports declared taps and form submission on
an existing view. SwiftUI does not publicly expose the original action closure
of an arbitrary `Button`, so the generic tap path cannot truthfully classify
keyboard, accessibility, pointer, or voice origin. It records `unknown` rather
than guessing. Exact all-modality command boundaries use the native Chill
control wrappers. No private API or appearance-changing primitive button style
is used to pretend otherwise.

`.event(_:when:)` observes an `Equatable` state projection but treats the value
only as a trigger. `.impression` derives a threshold crossing from SwiftUI's
public geometry and nearest scroll-view bounds, then runs bounded dwell work
that is cancelled when visibility, page exposure, or privacy becomes
ineligible. It records only the declared semantic element, role, threshold,
and duration. A blocked or disabled subtree schedules no impression work.

### Apple diagnostics

The first root page installs one process bridge only when diagnostic consent
and sampling permit capture. On iOS and macOS 27 it consumes the modern
`MetricManager` asynchronous metric and diagnostic report sequences. The
public `MXMetricManagerSubscriber` API is retained only as the deployment-floor
compatibility path on iOS 17–26 and macOS 14–26.

Chill emits bounded facts for crash, hang, CPU exception, excessive disk write,
slow launch, memory exception, and aggregate performance report availability.
It does not buffer or export call stacks, exception messages, device details,
raw measurements, or encoded MetricKit reports. Delivery is gated again by
the current diagnostic consent at record time.

## Consequences

- Representative SwiftUI code contains semantic declarations, not telemetry
  calls or lifecycle bookkeeping.
- Pages compose across navigation stacks and presentations with stable
  instance identities and per-surface focus.
- Native wrappers provide truthful committed control boundaries; generic
  modifiers deliberately avoid false input-origin precision.
- Disabled, blocked, denied, and sampled-out paths fail before capture work.
- UIKit adapters can share the core record and context model while owning their
  platform-specific navigation and command observation in CHILL-14.
