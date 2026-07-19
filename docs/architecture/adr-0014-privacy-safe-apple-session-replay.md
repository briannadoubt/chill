# ADR-0014: Apple replay captures redacted structure, not video

- Status: Accepted
- Date: 2026-07-15

## Context

Session replay must explain what a person could see and do without turning the
SDK into a screenshot recorder. Continuous pixels are expensive, difficult to
redact reliably, and incompatible with the zero-raw-sensitive-buffer budget.
Text, accessibility labels, entered values, image pixels, web content, media,
and custom drawing therefore cannot cross into a replay queue and become safe
later.

Replay also has stricter lifecycle and resource requirements than behavior
facts. It is independently consented and sampled, has a larger but hard storage
cap, needs time alignment with normal facts, and must remain reconstructable
after incremental coalescing or a process kill. Linking it must remain optional.

## Decision

### A separately linked automatic adapter

`ChillReplay` is an independently importable Swift product and is not linked by
the core or umbrella products. The application configures `ChillSessionReplay`
once; no event, gesture, or frame emission API is available to application
code. A 250 ms timer observes eligible native windows by default. App
background and termination boundaries seal the current chunk automatically.

AppKit uses a non-mutating local event monitor. UIKit-family platforms attach
non-cancelling simultaneous tap and pan observers to active windows. These
observers retain only quantized positions, phases, monotonic time, and an
eligible structural node ID. Scroll offsets and viewport/safe-area changes come
from the structural observation. The adapter never takes a screenshot, reads a
layer backing store, or serializes a view hierarchy description.

### Consent and policy precede traversal

The native timer checks the current runtime's independent replay consent and
sampling decision before walking a window. A denied replay class performs no
tree allocation or ID generation. Consent withdrawal discards the unsealed
prefix and deletes every pending encrypted replay chunk; sampling exclusion
discards only the unsealed prefix.

UIKit declarations contribute their monotone `privacy` and `replay` policy
directly. SwiftUI policy modifiers install an invisible, noninteractive,
zero-content platform probe with the subtree's effective policy. The replay
adapter reads only that probe's rectangle and policy. Nodes geometrically
inside a blocked region are omitted, while masked regions become opaque. This
avoids reflecting SwiftUI values or depending on private SwiftUI types.

### Source masking is an allowlist

The adapter creates a sendable replay node directly on the main actor. The node
contains a generated stable ID, parent/order, bounded semantic ID if explicitly
declared, native role/state, coarse geometry, visibility, focus, opacity, and
optional scroll state. Source content is represented as follows before the node
leaves the adapter:

| Source | Replay representation |
| --- | --- |
| Label or ordinary text input | `text` mask with `0`, `1-4`, `5-16`, `17-64`, or `65+` length bucket |
| Secure input | `secure_input` mask, regardless of configuration |
| Image | `pixels` opaque rectangle |
| Web or media surface | `pixels` opaque rectangle; descendants suppressed |
| Unknown application drawing | `custom_drawing` opaque rectangle; descendants suppressed |
| Known system container/control | structural role, state, geometry, and safe children only |

Labels, values, accessibility strings, pixel buffers, class descriptions, and
arbitrary application state do not appear in any replay model. Application
custom views fail closed as opaque rectangles. A future safe custom adapter
will require a separate review; `.replay(.automatic)` is not permission to read
custom pixels or text.

### Keyframes, deltas, and monotonic alignment

Every chunk starts with a full structural keyframe. Later observations contain
only changed/upserted nodes, removed IDs, viewport changes, or gestures. Exact
duplicates are coalesced. Admission retains one latest observation and at most
64 pending gestures, so an overloaded actor mailbox cannot grow without bound.
Geometry is rounded before admission.

Chunks use the runtime boot ID and a half-open monotonic interval. Gestures and
behavior facts therefore align without trusting wall-clock continuity. Rotation
occurs after ten seconds, 512 frames, or the memory budget. A rotated chunk
starts from a new full keyframe, so each encrypted blob reconstructs
independently. If a serialized chunk exceeds its cap, trailing deltas are
suppressed before the keyframe; a keyframe that cannot fit is dropped and
counted rather than split into an undecodable fragment.

### Protected local chunks and correlated OTLP metadata

The document is deterministic sorted-key JSON, gzip-compressed, then encrypted
with AES-256-GCM. The default key is generated with system cryptographic random
bytes and stored as an after-first-unlock, this-device-only Keychain item. Chunk
files use atomic replacement and iOS-family file protection. Recovery decrypts
and validates every immutable file; a wrong key, corrupt tag, malformed gzip,
or invalid document is quarantined without cleartext fallback.

Memory is capped at 16 MiB and disk at 256 MiB by contract; frugal defaults are
8 MiB and 64 MiB. Disk pressure evicts the oldest replay chunk and increments a
bounded counter. Upload acknowledgement removes exact chunk IDs. Public service
integration can read only the encrypted envelope, never decoded frames.

After a file is durable, the normal behavior runtime receives one replay
metadata fact containing chunk ID, encrypted digest/size, codec, session, boot,
and monotonic start. Actual replay bytes never enter OTLP Logs. The eventual
replay upload service uses a separate bounded binary route and backend
idempotency.

## Consequences

- Structural playback cannot reproduce arbitrary canvas or pixel-perfect media;
  those regions are intentionally opaque.
- Polling native geometry trades some fidelity for bounded CPU and energy. The
  adapter coalesces unchanged frames and may lower fidelity before UI work is
  affected.
- SwiftUI privacy remains declarative and does not rely on private reflection.
- Replay is at-least-once at the blob boundary; chunk IDs and digests make
  duplicate upload safe to deduplicate.
- AppKit, iOS, tvOS, and visionOS share the automatic native adapter. watchOS
  builds the data/storage product but has no window-tree adapter in v1.

## Verification

Release tests cover source canaries in labels, secure inputs, custom drawing,
blocked UIKit subtrees, and SwiftUI policy regions; keyframe/delta
reconstruction; monotonic rotation; gesture and viewport alignment; encryption
and wrong-key quarantine; crash recovery; exact acknowledgement; consent purge;
and disk pressure. The broader replay performance journey and physical-device
energy/jank measurements remain release gates from ADR-0006.
