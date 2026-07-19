# ADR-0011: UIKit declarations attach public native observers

- Status: Accepted
- Date: 2026-07-15
- Owners: Apple SDK architecture
- Scope ticket: CHILL-14

## Context

UIKit applications express navigation through mutable controller containers and
commands through target-action and gesture recognition. Chill must preserve
those native systems without requiring a tracking call, replacing application
delegates, globally swizzling lifecycle methods, or guessing one process-wide
window. Multi-window applications require every page, action, impression, and
lifecycle fact to stay with its owning `UIWindowScene`.

The same semantic contracts as SwiftUI apply: page paths are structured,
controller reuse does not manufacture identity, outer annotations win,
privacy restrictions are monotone, labels and entered values are not captured,
and macros invoked by native callbacks inherit UI context where the public
dispatch boundary permits it.

## Decision

The `ChillUIKit` product is independently importable and is re-exported by the
umbrella `Chill` product on UIKit platforms. Application code declares meaning
during ordinary view setup:

```swift
final class CatViewController: UIViewController {
  override func viewDidLoad() {
    super.viewDidLoad()

    annotation("cat.id", value: catID)
    page("cat")

    adoptButton
      .action("cat.adopt")
      .privacy(.redacted)

    heroView.impression("cat.hero", role: "image")
  }
}
```

These calls configure stable declarations. They do not emit behavior. There is
no public `track`, `record`, or `capture` function and no call is made at an
interaction site.

### Per-window page reconciliation

Each declared controller receives a zero-sized, noninteractive observer view.
Public view/window callbacks coalesce reconciliation on the next main-actor
turn. A coordinator associated with the exact owning `UIWindow` diffs:

- `UINavigationController.viewControllers` as a push chain;
- selected and retained `UITabBarController` branches;
- displayed `UISplitViewController` columns;
- ordinary child containment; and
- the presented-controller chain, classifying full-screen covers, sheets,
  popovers, and overlays.

Logical controller objects retain UUIDv7 page identity while live. Reordering,
focus, scene suspension, and visual callbacks update the same lifetime. Pop or
dismiss ends a page once, inner-first; reinserting a previously ended
controller creates a fresh navigation-entry identity. Starts are outer-first.

A surface should declare its root semantic page on the root container or root
controller, such as `navigationController.page("app")`. This gives tab and
split siblings one stable semantic root. Dynamic identifiers remain
annotations. The coordinator never walks `UIApplication`, connected scenes, or
`UIScreen.main`; it observes only the local controller, view, window, and
window scene.

Scene activation notifications are registered with the exact `UIWindowScene`
as their notification object. They produce bounded lifecycle facts and update
page exposure without conflating simultaneous windows.

### Annotations, privacy, and macro context

Associated declaration nodes own stable annotation-scope and element IDs.
Controller ancestry is resolved before descendant view ancestry, so an outer
value wins a collision. Reconfiguring the same key on one reusable object
updates that object's declaration; it does not create another hierarchy level.
Privacy and replay dispositions only become stricter toward descendants.

Ordinary `UIControl` instances use one public target-action observer at the
declared committed event: primary activation, value change, or return-key
submission. Gesture recognizers emit only when recognition ends successfully.
`UIEvent` is used when present to distinguish touch, indirect pointer,
keyboard, remote, and system input. A nil or ambiguous event is exported as
`unknown`, never guessed to be accessibility or voice.
The event object's in-memory identity and timestamp form a short-lived native
activation token in a bounded per-window set, preventing overlapping observer
events from emitting twice without exporting an address or timestamp. A
gesture over a closer declared control yields to that control.

`ChillButton`, `ChillSwitch`, `ChillSegmentedControl`, `ChillSlider`,
`ChillStepper`, and `ChillTextField` are optional native subclasses. They
override UIKit's documented `sendAction` hook solely to carry the current
annotation and page task-local through application callbacks. Consequently,
an `@Activity` or `@Event` function invoked by that callback correlates without
an application-authored telemetry call. Existing controls still emit actions
automatically after `.action(...)`; the subclasses add exact callback-context
propagation.

### Impressions and reuse

Impressions intersect the declared view's converted bounds with its window and
each clipping or scroll ancestor. Scroll offset observation and public view
attachment callbacks cancel or begin bounded dwell work. Occluded, retained,
blocked, disabled, hidden, transparent, or off-viewport views do no capture
work. Only the semantic element, role, threshold, and duration are recorded.
Per-window weak registries make page-transition work proportional to declared
impressions; Chill never scans or reflects over unrelated view subtrees.

`ChillTableViewCell`, `ChillTableHeaderFooterView`,
`ChillCollectionViewCell`, and `ChillCollectionReusableView` reset annotation,
element, and pending dwell state throughout the reusable view subtree at
UIKit's native `prepareForReuse` boundary. Static action, impression, and
privacy declarations remain installed, while a new item cannot inherit the
previous item's annotations or element ID.

## Consequences

- UIKit adoption is a handful of declarations in existing setup code, not an
  analytics SDK spread across interaction callbacks.
- Native container state produces the same structured path and exposure model
  as SwiftUI while remaining scene-local.
- No delegate proxy, global swizzle, application singleton, screen singleton,
  label reflection, or private API is required.
- Exact action facts work with existing native controls; optional native Chill
  subclasses also propagate macro context through target dispatch.
- Header/footer and cell reuse have an explicit automatic semantic lifetime.
- Representative simulator tests cover action eligibility, outer-first scope,
  reuse reset, page exposure/focus, navigation path composition, and exactly-once
  pop termination.
