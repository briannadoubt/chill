#if os(iOS)
  import ChillCore
  import Testing
  import UIKit
  import os

  @testable import ChillUIKit

  private final class UIKitCollectingSink: RecordSink {
    private let storage = OSAllocatedUnfairLock(
      initialState: [BehaviorRecord]()
    )

    func submit(_ record: BehaviorRecord) {
      storage.withLock { $0.append(record) }
    }

    var records: [BehaviorRecord] {
      storage.withLock { $0 }
    }
  }

  @MainActor
  private final class UIKitActivityTarget: NSObject {
    @objc func submit() {
      _ChillInstrumentation.withActivity(
        name: "adoption.submit",
        kind: .domain,
        role: .operation
      ) {}
    }
  }

  @Suite("UIKit declarative instrumentation", .serialized)
  @MainActor
  struct UIKitInstrumentationTests {
    @Test("Native controls emit once with outer-first annotations")
    func controlAction() throws {
      let sink = try configureUIKitRuntime()
      defer { Chill.disable() }
      let outer = UIView()
        .annotation("account.tier", value: "internal")
      let button = UIButton(type: .system)
        .annotation("account.tier", value: "descendant")
        .annotation("cat.id", value: "cat-42")
        .action("cat.adopt")
      outer.addSubview(button)

      semanticNode(for: button)?.controlObserver?.receiveActivation(
        button,
        event: nil
      )

      let record = try #require(sink.records.only)
      #expect(record.kind == .action)
      #expect(
        record.annotations.values[try AnnotationName("account.tier")]
          == .string("internal")
      )
      #expect(
        record.annotations.values[try AnnotationName("cat.id")]
          == .string("cat-42")
      )
      guard case .action(let payload) = record.payload else {
        Issue.record("Expected action payload")
        return
      }
      #expect(payload.elementID.rawValue == "cat.adopt")
      #expect(payload.input == .unknown)
    }

    @Test("Disabled controls and blocked subtrees emit nothing")
    func actionEligibility() throws {
      let sink = try configureUIKitRuntime()
      defer { Chill.disable() }
      let blocked = UIView().privacy(.blocked)
      let button = UIButton(type: .system).action("cat.adopt")
      blocked.addSubview(button)

      semanticNode(for: button)?.controlObserver?.receiveActivation(
        button,
        event: nil
      )
      button.isEnabled = false
      semanticNode(for: button)?.controlObserver?.receiveActivation(
        button,
        event: nil
      )

      #expect(sink.records.isEmpty)
    }

    @Test("Accessibility activation uses the native semantic boundary")
    func accessibilityActivation() throws {
      let sink = try configureUIKitRuntime()
      defer { Chill.disable() }
      let secretLabel = "SECRET-ACCESSIBILITY-CANARY"
      let button = ChillButton(type: .system)
        .annotation("cat.id", value: "cat-42")
        .action("cat.adopt")
      button.setTitle("Request adoption", for: .normal)
      button.accessibilityLabel = secretLabel

      #expect(button.isAccessibilityElement)
      semanticNode(for: button)?.controlObserver?.receiveActivation(
        button,
        event: nil
      )

      let record = try #require(sink.records.only)
      #expect(record.kind == .action)
      #expect(record.name.rawValue == "cat.adopt")
      #expect(record.name.rawValue != secretLabel)
      #expect(
        record.annotations.values[try AnnotationName("cat.id")]
          == .string("cat-42")
      )
    }

    @Test("Reusable views reset identity and stale annotations")
    func reuseBoundary() {
      let cell = ChillTableViewCell()
        .annotation("cat.id", value: "cat-1")
        .impression("cat.row", role: "row")
      let button = UIButton(type: .system).action("cat.open")
      cell.contentView.addSubview(button)
      let node = semanticNode(for: cell)!
      let buttonNode = semanticNode(for: button)!
      let firstElementID = node.elementInstanceID
      let firstButtonElementID = buttonNode.elementInstanceID

      cell.prepareForReuse()

      #expect(node.elementInstanceID != firstElementID)
      #expect(buttonNode.elementInstanceID != firstButtonElementID)
      #expect(node.declarations.isEmpty)
      #expect(node.impression?.name.rawValue == "cat.row")
      #expect(buttonNode.action?.name.rawValue == "cat.open")
    }

    @Test("Cached page context does not retain its controller node")
    func pageContextOwnership() {
      weak var retainedController: UIViewController?
      weak var retainedNode: UIKitSemanticNode?
      do {
        let controller = UIViewController().page("cat")
        let node = semanticNode(for: controller)!
        node.pageState?.contextNodes = [node]
        retainedController = controller
        retainedNode = node
      }

      #expect(retainedController == nil)
      #expect(retainedNode == nil)
    }

    @Test("Surface graph preserves native container exposure and focus")
    func surfaceGraph() throws {
      let graph = UIKitPageSurfaceGraph()
      let root = try makePath(
        instances: ["root"],
        segments: ["app"],
        relation: .root
      )
      let pushed = try makePath(
        instances: ["root", "cat"],
        segments: ["app", "cat"],
        relation: .push
      )
      graph.register(
        path: root,
        parentInstanceID: nil,
        declaredExposure: .foreground,
        declaredFocused: true
      )
      graph.register(
        path: pushed,
        parentInstanceID: root.instanceID,
        declaredExposure: .foreground,
        declaredFocused: true
      )

      #expect(graph.resolve(root).exposure == .retained)
      #expect(graph.resolve(root).focused == false)
      #expect(graph.resolve(pushed).exposure == .foreground)
      #expect(graph.resolve(pushed).focused == true)
    }

    @Test("Native activation IDs are deduplicated with a bounded window scope")
    func nativeActivationDeduplication() throws {
      let sink = try configureUIKitRuntime()
      defer { Chill.disable() }
      let coordinator = UIKitSurfaceCoordinator(window: UIWindow())
      let event = NSObject()
      let token = UIKitNativeActivationToken(
        eventIdentity: ObjectIdentifier(event),
        timestampBits: 42
      )
      let later = UIKitNativeActivationToken(
        eventIdentity: ObjectIdentifier(event),
        timestampBits: 43
      )

      #expect(coordinator.claimNativeActivation(token))
      #expect(!coordinator.claimNativeActivation(token))
      #expect(coordinator.claimNativeActivation(later))
      let distinctEvents = (0..<129).map { _ in NSObject() }
      for (index, distinctEvent) in distinctEvents.enumerated() {
        #expect(
          coordinator.claimNativeActivation(
            UIKitNativeActivationToken(
              eventIdentity: ObjectIdentifier(distinctEvent),
              timestampBits: UInt64(index + 100)
            )
          )
        )
      }
      #expect(coordinator.claimNativeActivation(token))
      UIKitFactEmitter.duplicateActionObservation(
        context: UIKitFactContext(
          annotations: .empty,
          page: nil,
          policy: .standard
        )
      )
      let diagnostic = try #require(sink.records.only)
      #expect(diagnostic.name.rawValue == "action.duplicate_observation")
      #expect(diagnostic.captureClass == .diagnostic)
    }

    @Test("Navigation stacks compose paths and end popped pages once")
    func navigationReconciliation() throws {
      let sink = try configureUIKitRuntime()
      defer { Chill.disable() }
      let navigation = UINavigationController()
        .annotation("account.tier", value: "internal")
        .page("app")
      let home = UIViewController()
        .page("home")
      let cat = UIViewController()
        .annotation("account.tier", value: "descendant")
        .annotation("cat.id", value: "cat-42")
        .page("cat")
      navigation.setViewControllers([home, cat], animated: false)
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
      window.rootViewController = navigation
      let coordinator = surfaceCoordinator(for: window)!

      coordinator.reconcile()

      let starts = sink.records.filter {
        $0.kind == .page && $0.operation == .start
      }
      #expect(starts.count == 3)
      let catStart = try #require(
        starts.first {
          $0.name.rawValue == "cat"
        })
      #expect(catStart.page?.segments.map(\.rawValue) == ["app", "home", "cat"])
      #expect(catStart.page?.relation == .push)
      #expect(
        catStart.annotations.values[try AnnotationName("account.tier")]
          == .string("internal")
      )

      navigation.setViewControllers([home], animated: false)
      coordinator.reconcile()
      let catEnds = sink.records.filter {
        $0.kind == .page && $0.operation == .end
          && $0.name.rawValue == "cat"
      }
      #expect(catEnds.count == 1)
      coordinator.reconcile()
      #expect(
        sink.records.filter {
          $0.kind == .page && $0.operation == .end
            && $0.name.rawValue == "cat"
        }.count == 1
      )
    }

    @Test("Contextual controls carry UI context into generated activities")
    func contextualControl() throws {
      let sink = try configureUIKitRuntime()
      defer { Chill.disable() }
      let root = UIViewController()
        .annotation("cat.id", value: "cat-42")
        .page("cat")
      let button = ChillButton(type: .system).action("cat.adopt")
      root.view.addSubview(button)
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
      window.rootViewController = root
      surfaceCoordinator(for: window)?.reconcile()

      withUIKitControlContext(control: button) {
        _ChillInstrumentation.withActivity(
          name: "adoption.submit",
          kind: .domain,
          role: .operation
        ) {}
      }

      let activities = sink.records.filter { $0.kind == .activity }
      let catID = try AnnotationName("cat.id")
      #expect(activities.map(\.operation) == [.start, .end])
      #expect(activities.allSatisfy { $0.page?.segments.last?.rawValue == "cat" })
      #expect(
        activities.allSatisfy {
          $0.annotations.values[catID] == .string("cat-42")
        }
      )
    }

    @Test("One control dispatch correlates its action and child activity")
    func contextualControlTrace() throws {
      let sink = try configureUIKitRuntime()
      defer { Chill.disable() }
      let button = ChillButton(type: .system).action("cat.adopt")
      let target = UIKitActivityTarget()

      // A Swift package test has no UIApplicationMain, so UIKit deliberately
      // declines to dispatch target-actions. Exercise the same boundary that
      // ChillButton wraps around native dispatch without requiring a host app.
      withUIKitControlContext(control: button) {
        semanticNode(for: button)?.controlObserver?.receiveActivation(
          button,
          event: nil
        )
        target.submit()
      }

      let action = try #require(sink.records.first { $0.kind == .action })
      let activities = sink.records.filter { $0.kind == .activity }
      #expect(activities.map(\.operation) == [.start, .end])
      #expect(action.trace?.traceID == activities[0].trace?.traceID)
      #expect(action.trace?.spanID == activities[0].trace?.parentSpanID)
      #expect(activities[0].trace == activities[1].trace)
    }

    @Test("Representative UIKit declarations require no tracking calls")
    func representativeSurfaceCompiles() {
      let root = UINavigationController()
        .annotation(["account.tier": "internal"])
        .page("app")
      let child = UIViewController()
        .annotation("cat.id", value: "cat-42")
        .page("cat")
      child.view.addSubview(
        UIButton(type: .system)
          .action("cat.adopt")
          .privacy(.redacted)
      )
      root.setViewControllers([child], animated: false)

      #expect(root.viewControllers == [child])
    }

    private func configureUIKitRuntime() throws -> UIKitCollectingSink {
      Chill.disable()
      let sink = UIKitCollectingSink()
      let runtime = ChillRuntime(
        configuration: try RuntimeConfiguration(
          sessionID: try SessionID("session-uikit"),
          bootID: try BootID("boot-uikit"),
          privacy: PrivacyPolicy(
            version: "test",
            analytics: .granted,
            diagnostic: .granted,
            annotationAllowlist: [
              "account.tier": .internalData,
              "cat.id": .pseudonymousIdentifier,
            ]
          ),
          sampling: SamplingConfiguration(
            stableKey: "uikit-tests",
            salt: "uikit-tests"
          )
        ),
        sink: sink
      )
      Chill.configure(runtime)
      return sink
    }

    private func makePath(
      instances: [String],
      segments: [String],
      relation: PageRelation
    ) throws -> PagePath {
      let ids = try instances.map(PageInstanceID.init)
      return try PagePath(
        surfaceID: SurfaceID("surface"),
        instanceID: ids.last!,
        instanceIDs: ids,
        segments: try segments.map(PageSegment.init),
        relation: relation,
        exposure: .foreground,
        focused: true
      )
    }
  }

  extension Array {
    fileprivate var only: Element? { count == 1 ? first : nil }
  }
#endif
