# ADR-0002: Outer-first annotation resolution

- Status: Accepted
- Date: 2026-07-14
- Scope ticket: CHILL-4
- Reference implementation: `contracts/annotations/v1/reference.py`

## Context

Chill annotations add semantic context to automatically observed behavior. A
declaration on an application root, scene, page, container, or element can use
the same key as a declaration below it. The product decision is intentionally
root-authoritative: an inner declaration cannot overwrite a value already
established by an outer scope.

The rule must remain deterministic across SwiftUI value reconstruction, UIKit
reuse, navigation and presentation containers, Android Compose, React, and the
browser DOM. It must also avoid putting unstable or high-frequency data into
SwiftUI environment propagation.

## Decision

An **annotation scope** is a semantic node instance containing an ordered list
of declarations. A platform adapter produces one **ancestry chain** ordered
from the logical root to the observed element. The resolver walks the chain
once from outermost to innermost and accepts the first declaration of each key.

```text
resolve(scopes_outer_to_inner):
    effective = empty map
    origins = empty map
    collisions = empty list

    for scope in scopes_outer_to_inner:
        for declaration in scope.declaration_order:
            if declaration.key is absent from effective:
                effective[declaration.key] = freeze(declaration.value)
                origins[declaration.key] = declaration.origin
            else:
                collisions.append(
                    key = declaration.key,
                    winner = origins[declaration.key],
                    loser = declaration.origin,
                    identical = same_type_and_value(
                        effective[declaration.key],
                        declaration.value
                    )
                )

    return immutable(effective, origins, collisions)
```

Resolution is `O(d)` in the number of declarations on the ancestry chain. The
result is an immutable snapshot. SDK implementations may use persistent maps
and structural sharing so an unchanged ancestor context is not repeatedly
copied.

### What “outer” means

Outer is defined by the logical runtime ancestry, not object allocation time,
render order, callback order, screen coordinates, or which record happens to
arrive first.

The broad order is:

1. trusted server-stamped tenant context;
2. process and application root;
3. window, scene, or browser document;
4. presented or navigated page ancestry;
5. containers and reusable item instances; and
6. the observed element.

Tenant fields are not ordinary developer annotations and cannot be declared by
an app. Resource attributes and project policy are also resolved separately
before user annotations are accepted.

Each platform adapter must supply a single chain. The shared resolver never
attempts to infer ancestry from timestamps or view memory addresses.

### SwiftUI modifier ordering

SwiftUI modifiers wrap the value produced before them. For declarations on the
same view expression, a later modifier is structurally outer:

```swift
CatView()
    .annotation("cat.id", value: "inner")
    .annotation("cat.id", value: "outer")
```

The effective value is `outer`. Chill emits a collision diagnostic for the
ignored `inner` declaration. Developers should prefer one annotation set per
semantic boundary when several keys are known together:

```swift
CatView()
    .annotation([
        "cat.id": cat.id,
        "cat.kind": cat.kind,
    ])
```

A map cannot contain a duplicate key. If a language-level annotation builder
does accept an ordered list with duplicate keys inside one atomic scope, its
first declaration wins and later duplicates produce diagnostics.

SwiftUI integration uses a small immutable value in `EnvironmentValues` with a
stable empty default. The value stores data only; it contains no closures or
function values. Gesture coordinates, scroll offsets, animation progress, and
other high-frequency values do not flow through the environment. Automatic
capture sends those directly to the bounded runtime or replay buffer without
invalidating an annotation-reading subtree on every frame.

### Parent and child example

```swift
VStack {
    CatView()
        .annotation("id", value: "child")
}
.annotation("id", value: "parent")
```

The observed `CatView` receives `id = parent`. The child attempt is retained
only as a collision diagnostic. Namespaced keys such as `cat.id` are still
preferred because a generic `id` is easy to establish accidentally at a broad
scope.

### Siblings

Siblings do not inherit from each other and have no precedence relationship.
Given a root with `Left` and `Right`, the left chain is `root -> left` and the
right chain is `root -> right`. Resolving one chain must never inspect the other.

### Navigation and presentations

A pushed destination, selected tab, sheet, popover, split-view column, or
overlay is a descendant of the logical scope that presented or owns it, even
when the framework renders it in a separate host tree. The platform adapter
provides that logical parent link.

While a presentation remains active, changes to a still-active ancestor create
a new effective snapshot for future facts. Records already emitted do not
change. If the winning ancestor declaration disappears, the first remaining
inner declaration becomes effective for subsequent snapshots and a context
change diagnostic is produced.

Navigation path structure is defined separately by CHILL-5. Annotation
precedence follows the same logical page ancestry but does not flatten or parse
the page path.

### Reused and virtualized views

A scope belongs to a semantic node **instance**, not a SwiftUI `View` value,
UIKit object address, Compose recomposition, React component function call, or
DOM recycling implementation detail.

Lists and grids must bind each visible item scope to the framework's stable
item identity plus its current presentation lifetime. Reusing a native cell or
reconstructing a value view cannot carry annotations from the prior item.
When an identity changes, the old scope is closed and a new scope is created.

### Dynamic values and snapshot timing

Annotation declarations may depend on ordinary application state. The resolver
creates an immutable effective snapshot when a behavior fact occurs.

- An impression snapshots context when its visibility policy is satisfied.
- An action snapshots context at native activation.
- A page snapshots context independently for each lifecycle fact.
- A macro activity snapshots context at start and retains that snapshot through
  its end, error, cancellation, child spans, and async task hops.
- A domain event snapshots context when the generated macro code observes it.

Changing an annotation later never rewrites a prior record or an already-started
activity. Rapidly changing values are not valid annotations; replay or a
specialized bounded signal records them.

### Collision diagnostics

Collisions never change the effective value. The resolver produces:

| Code | Condition | Default severity |
| --- | --- | --- |
| `annotation.shadowed` | An inner declaration has a different type or value | warning in debug, diagnostic in production |
| `annotation.redundant` | An inner declaration repeats the exact type and value | debug |
| `annotation.invalid_key` | A key violates the canonical key grammar | warning and declaration dropped |
| `annotation.reserved_key` | App code attempts to use a trusted namespace | error and declaration dropped |

Diagnostics contain the key, scope identities, depths, and declaration indexes,
but never annotation values. SDKs throttle the same collision tuple once per
scope lifetime. Production diagnostic sampling and export are project policy.

### Privacy and policy are not ordinary annotations

Outer-first precedence protects root-established identity and experiment
context, but privacy cannot use a simple winner-takes-all rule. Capture policy
uses a separate monotone restriction lattice: a descendant may make collection
more restrictive, but never less restrictive. For example, `blocked` is more
restrictive than `redacted`, which is more restrictive than `allowed`.

This prevents either an outer convenience default or an inner component from
weakening masking, consent, or collection policy. Annotation values are still
filtered through the resulting policy and schema registry before they enter an
effective snapshot.

## Required algebraic properties

Every SDK implementation must satisfy these properties:

1. **Outer stability:** appending descendants cannot change an existing key.
2. **First-source selection:** a key resolves to the first declaration in the
   root-to-leaf chain.
3. **Key independence:** collisions on one key cannot change another key.
4. **Sibling isolation:** declarations in one sibling cannot affect another.
5. **Determinism:** identical chains produce byte-equivalent snapshots and
   diagnostics.
6. **Removal promotion:** removing a winning declaration promotes the first
   remaining declaration for future snapshots.
7. **Exact collision accounting:** every declaration after the winner produces
   exactly one collision.
8. **Snapshot immutability:** mutating a caller-owned collection after
   resolution cannot change the resolved value.
9. **Type-sensitive redundancy:** `false`, `0`, and `0.0` are distinct values
   for collision classification even when a host language compares them equal.

The executable reference model and exhaustive small-domain property tests are
under `contracts/annotations/v1` and `tests/contracts`.

## Consequences

- Broad keys are intentionally authoritative; developers should namespace them
  and keep broad scopes small.
- Component authors cannot locally repair an incorrect outer value. Debugging
  must point to the winning origin clearly.
- Platform adapters carry logical scope identity across framework-specific
  presentation boundaries and reuse.
- The SwiftUI environment carries stable semantic data only, preserving view
  identity and avoiding closure or per-frame invalidation traps.
- Privacy, trusted tenant context, and resource merging remain separate layers
  with stricter rules than developer annotations.
