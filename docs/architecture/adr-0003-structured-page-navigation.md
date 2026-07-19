# ADR-0003: Structured page and navigation composition

- Status: Accepted
- Date: 2026-07-14
- Scope ticket: CHILL-5
- Reference implementation: `contracts/navigation/v1/reference.py`

## Context

Chill needs useful page analytics without asking an application developer to
call `Tracker.track()` during every navigation transition. A declaration such
as `.page("cat")` must compose naturally when pages are nested, pushed,
selected in tabs, presented, or displayed concurrently. A flattened route
string cannot represent split views, concurrent windows, retained tab stacks,
or the difference between a live back navigation and process restoration.

Page names also cannot include database identifiers or other unbounded values.
Those values belong in annotations, while route identity still needs enough
private information for an SDK to decide whether a live entry was preserved or
replaced.

## Decision

Chill models navigation as a **rooted page graph per surface**. A surface is one
independently navigable window, scene, browser document, Android activity
host, or equivalent UI root. There is no process-wide current page.

Every live page entry has:

- a UUIDv7 `page_instance_id`, unique for one page-entry lifetime;
- a stable, low-cardinality semantic segment declared by `.page("name")`;
- an opaque SDK-local route identity used only for reconciliation;
- a logical parent page instance;
- a relation to that parent;
- an exposure state; and
- an optional focused state.

The exported path for an entry is the pair of aligned arrays obtained by
walking its logical ancestry from root to that entry:

```json
{
  "path": ["app", "cats", "cat"],
  "path_instance_ids": [
    "019b6a80-1000-7000-8000-000000000001",
    "019b6a80-1000-7000-8000-000000000002",
    "019b6a80-1000-7000-8000-000000000003"
  ]
}
```

The arrays always have equal length, and the final instance ID is the page
record's `instance_id`. Arrays are the source of truth. A UI may render them as
`app / cats / cat`, but an SDK, collector, or query must never serialize and
later parse a delimiter-concatenated path.

### Semantic declarations and private route identity

`.page("cat")` contributes the literal segment `cat`. A dynamic entity value is
an annotation, not part of the segment:

```swift
CatScreen(cat: cat)
    .page("cat")
    .annotation("cat.id", value: cat.id)
```

A native router may reconcile the destination with a private key equivalent to
`cat:<id>`. That private key never becomes the page name or path. Replacing cat
123 with cat 999 therefore ends one `cat` instance and starts another while
both have the same semantic path.

Nested `.page` declarations add boundaries from outer to inner. Containers
without a declaration add no segment. Chill may report a development
diagnostic for an observed destination with no semantic declaration, but it
must not silently use localized titles, type descriptions, object addresses,
URLs containing parameters, or accessibility text as a production page name.

### Relations

The relation is structural and has one of these values:

| Relation | Meaning |
| --- | --- |
| `root` | The single root of a surface |
| `push` | A member added above another entry in a LIFO stack |
| `tab` | The root of a retained selectable tab branch |
| `split` | A column or pane that may be exposed with siblings |
| `sheet` | A modal sheet that leaves some presenter pixels exposed |
| `popover` | An anchored presentation that leaves the presenter exposed |
| `overlay` | An in-surface semantic overlay page |
| `cover` | A presentation that fully occludes its presenter |

The relation is not inferred from the semantic name. A presentation is a
logical child of its presenter even if a UI framework mounts it in a separate
render tree or host controller.

### Exposure and focus

Page lifetime, pixel exposure, and focus are distinct concepts:

| Exposure | Meaning |
| --- | --- |
| `foreground` | Rendered and eligible for interaction |
| `visible` | At least partially rendered but not the foreground target |
| `occluded` | Mounted but meaningfully hidden by another presentation |
| `retained` | Kept in navigation state but offscreen |

Several pages can be `foreground` in a split layout. A surface has at most one
focused page, and a focused page must be foreground. Focus is a deterministic
hint for selecting a convenience **primary page**; it does not erase the other
exposed pages.

An exposed-page query returns every `foreground` or `visible` page path. A
primary-page query returns at most one page per surface, choosing the focused
page first and otherwise applying a stable ordering based on foreground state,
presentation layer, framework order, depth, and instance ID. Warehousing keeps
the complete lifecycle facts rather than only this convenience projection.

### Page-instance lifecycle

A page instance begins when a logical route entry is created, even if a router
preloads it in the retained state. Its semantic segment, private route identity,
logical parent, and page instance ID are immutable.

Chill emits:

- `page.start` when the entry is created;
- `page.update` when exposure, focus, or its derived path snapshot changes; and
- `page.end` when the entry is removed or its surface is destroyed.

An exposure update is not a new page start. A page impression is a separate
policy-controlled fact and is not implied merely because an entry exists.

Atomic navigation transactions order lifecycle facts as follows:

1. end removed descendants from inner to outer;
2. update preserved ancestors from outer to inner;
3. start new descendants from outer to inner; and
4. emit the navigation transition fact that references the resulting graph.

The records share the transaction's navigation ID and device sequence provides
the final ordering. Records already exported are immutable.

### Operation semantics

#### Stack push and back

A push retains the previous top entry and starts a new foreground entry. Back
ends the popped entry and exposes the predecessor with the **same** instance ID.
Revealing a retained predecessor never emits a second start.

Stack replacement computes the longest live common prefix using private route
identity, semantic segment, and relation. The common prefix keeps its instance
IDs. The old suffix ends and the new suffix receives fresh IDs. A semantic name
match alone is not enough to preserve identity.

#### Tabs

Each tab owns a retained branch, including its own push history. Selecting a
different tab changes exposure and focus only. It neither ends the old branch
nor restarts the selected branch. If an application actually discards an
inactive tab's state, the discarded page entries end normally.

#### Sheets, popovers, overlays, and covers

A presentation starts a child branch above its presenter. Sheets, popovers, and
overlays leave the presenter `visible`; a cover makes it `occluded`. The
presentation is foreground and focused. Nested presentations repeat this rule.
Dismissal ends the presented subtree from inner to outer and returns the
existing presenter to foreground without restarting it.

If a platform can prove that a presentation completely hides the presenter,
it may report `occluded` instead of `visible`; this is an exposure observation,
not a different relation.

#### Split views and adaptive collapse

A split container may expose several sibling paths simultaneously. For example,
the visible set can contain both `["app", "cats"]` and `["app", "cat"]`.
Collapsing to a compact layout changes exposure; it does not end a retained
column. Expanding exposes the same instances again. A page ends only if the
application or framework removes that logical entry rather than retaining it.

#### Deep links

A deep link is applied as one graph transaction. During a live process it
preserves the longest exact route-entry prefix and replaces the remainder. A
cold deep link has no live prefix and creates every entry. Intermediate UI
frames produced while the platform commits the transaction are not separate
semantic pages unless they become stable application navigation state.

#### Restoration and process death

Persisted navigation state contains route specifications, never Chill page
instance IDs. Restoration after process or document death creates new instance
IDs for every restored entry and uses the `restore` lifecycle cause. This
prevents one logical page lifetime from spanning a crash, upgrade, or cold
launch.

Temporary suspension that preserves the live UI object graph—application
backgrounding, an inactive scene, browser page freeze, or back-forward cache—
keeps instance IDs. It changes exposure only if the platform can observe a
meaningful change.

#### Concurrent presentations and windows

Presentation branches can nest, but the platform adapter serializes mutations
to a surface so each exported graph snapshot is valid. Separate surfaces are
independent and can each have a primary page at the same time. A global
`currentPage` API is therefore forbidden in the canonical model.

### SwiftUI mapping

The Swift SDK uses macros to generate declarations and diagnostics, while the
runtime observes value-driven native navigation state. Applications should use
`NavigationStack(path:)` and `NavigationSplitView` with stable, lightweight,
`Hashable` route values. `NavigationView` is not part of the Chill integration
surface.

```swift
@MainActor
@Observable
final class Router {
    var path: [Route] = []
}

enum Route: Hashable, Codable {
    case cat(id: String)
    case settings
}

struct AppRoot: View {
    @State private var router = Router()

    var body: some View {
        NavigationStack(path: $router.path) {
            CatList()
                .page("cats")
                .navigationDestination(for: Route.self) { route in
                    switch route {
                    case .cat(let id):
                        CatDestination(id: id)
                            .page("cat")
                            .annotation("cat.id", value: id)
                    case .settings:
                        Settings()
                            .page("settings")
                    }
                }
        }
        .page("app")
    }
}
```

The route value supplies private reconciliation identity; `.page` supplies the
exported name. The macro/runtime combination must not require an imperative
tracking call, inject closure-valued environment keys, or propagate per-frame
geometry through `EnvironmentValues`. It preserves SwiftUI structural identity
and keys navigation entries by stable route identity, never array indices or a
freshly generated ID during `body` evaluation.

Navigation initiated with APIs that do not expose state is captured on a
best-effort visual lifecycle basis. The SDK emits a development diagnostic when
it cannot establish stable route-entry identity or exact restoration behavior.
Apple's stateful path APIs are the precise integration route because the path
is both navigation state and the basis for deep linking/restoration.

### Other platform mappings

- **UIKit:** diff `UINavigationController.viewControllers`, selected
  `UITabBarController` branches, `UISplitViewController` columns, and presented
  controller chains using logical controller identity. Controller reuse alone
  does not imply a new page instance.
- **Android:** map an app-owned Navigation 3 back-stack key to private route
  identity, or observe `NavController` destination/back-stack entries for the
  stable entry lifetime. Multiple back stacks map to retained tab branches;
  adaptive panes map to split relations.
- **Web:** observe router and History API transactions. `pushState` creates an
  entry, back/forward reveals the live history entry, and `replaceState` uses
  route reconciliation. A reload creates new Chill instance IDs. Framework
  adapters provide stable router keys where available; URLs are never used as
  unfiltered semantic page names.

Platform adapters may produce different transient framework callbacks, but the
exported graph, paths, and lifecycle delta must match the executable reference.

## Required invariants

Every SDK must satisfy these invariants:

1. **Structured source:** page paths are arrays and are never parsed from a
   flattened display string.
2. **Aligned ancestry:** `path` and `path_instance_ids` have equal nonzero
   length; the last ID equals `instance_id`.
3. **One root:** every live surface has exactly one root entry.
4. **Acyclic graph:** parent links exist on the same surface and form no cycle.
5. **Stable live identity:** visibility, focus, tab selection, adaptive layout,
   back reveal, and suspension do not manufacture new instance IDs.
6. **Fresh restored identity:** cold restoration never reuses persisted Chill
   page instance IDs.
7. **Exact replacement:** live replacement preserves exactly the longest equal
   route-entry prefix and replaces the suffix.
8. **Low-cardinality path:** dynamic entity values remain private route
   identity and/or annotations, not semantic segments.
9. **Complete visibility:** every foreground or partially visible page is
   represented; split and presentation layouts are not collapsed to one page.
10. **Per-surface focus:** a surface has at most one focused page, and it is
    foreground.
11. **Determinism:** the same valid graph yields the same exposed and primary
    paths independent of framework callback ordering.
12. **Ordered lifecycle:** descendants end inner-first, new descendants start
    outer-first, and preserved pages update without restarting.

The executable reference model and conformance/property tests live under
`contracts/navigation/v1` and `tests/conformance`.

## Consequences

- A query can group reliably by semantic path while annotations retain entity
  context.
- Session replay can associate pixels with every exposed page rather than only
  a guessed current screen.
- Native SDKs need platform-specific graph adapters but share one observable
  contract.
- Restoration analytics count a new runtime page lifetime, while live back and
  tab navigation preserve the lifetime users actually resumed.
- Developers get declarative page names and can keep using native navigation;
  no imperative tracking call is introduced.

## Primary platform references

- Apple, [Understanding the navigation stack](https://developer.apple.com/documentation/swiftui/understanding-the-navigation-stack)
- Apple, [NavigationStack](https://developer.apple.com/documentation/swiftui/navigationstack)
- Apple, [NavigationSplitView](https://developer.apple.com/documentation/swiftui/navigationsplitview)
- Apple, [Migrating to new navigation types](https://developer.apple.com/documentation/swiftui/migrating-to-new-navigation-types)
- Android, [Back stack](https://developer.android.com/guide/navigation/backstack)
- Android, [Navigation 3](https://developer.android.com/guide/navigation/navigation-3)
- WHATWG, [HTML Living Standard: session history and navigation](https://html.spec.whatwg.org/multipage/nav-history-apis.html)
