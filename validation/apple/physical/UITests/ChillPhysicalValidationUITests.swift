import XCTest

final class ChillPhysicalValidationUITests: XCTestCase {
  private var app: XCUIApplication!

  override func setUp() {
    continueAfterFailure = false
    app = XCUIApplication()
    addUIInterruptionMonitor(withDescription: "System alerts") { alert in
      for label in ["Later", "Not Now", "Cancel", "Close", "Remind Me Later"] {
        let button = alert.buttons[label]
        if button.exists {
          button.tap()
          return true
        }
      }
      return false
    }
  }

  func testInteractionEvidence() {
    app.launchArguments = ["--physical-validation-smoke"]
    app.launch()

    let list = app.descendants(matching: .any)["validation.list"]
    XCTAssertTrue(list.waitForExistence(timeout: 15))
    attachState(name: "before-interaction")

    let juniper = app.buttons["cat.cat-123"]
    XCTAssertTrue(juniper.waitForExistence(timeout: 5))
    juniper.tap()

    attachState(name: "after-navigation")
    let adopt = app.buttons["Request adoption"]
    XCTAssertTrue(adopt.waitForExistence(timeout: 5))
    XCTAssertTrue(adopt.isHittable)
    adopt.tap()
    XCTAssertEqual(app.state, .runningForeground)
  }

  func testReleaseEnvironmentPreflight() throws {
    try requireReleaseGate()
    app.launch()
    let badge = app.descendants(matching: .any)["validation.release-environment"]
    XCTAssertTrue(badge.waitForExistence(timeout: 15))
    guard let value = badge.value as? String else {
      XCTFail("release environment badge did not expose a value")
      return
    }
    XCTAssertTrue(value.contains("physical=true"), value)
    XCTAssertTrue(value.contains("thermal=nominal"), value)
    XCTAssertTrue(value.contains("power=unplugged"), value)
    XCTAssertTrue(value.contains("os=17.0"), value)
  }

  func testReleaseColdLaunchSingleSample() throws {
    try requireReleaseGate()
    let options = XCTMeasureOptions()
    options.iterationCount = 1
    measure(
      metrics: [XCTApplicationLaunchMetric(waitUntilResponsive: true)],
      options: options
    ) {
      app.launch()
      app.terminate()
    }
  }

  func testCoreHotPathWorkload() throws {
    try requireReleaseGate()
    launchReplayWorkload()
    let burst = app.buttons["queue.emit-burst"]
    XCTAssertTrue(burst.waitForExistence(timeout: 10))
    let options = singleIterationOptions()
    measure(metrics: [XCTClockMetric()], options: options) {
      burst.tap()
    }
    XCTAssertEqual(app.state, .runningForeground)
  }

  func testRepresentativeSustainedWorkload() throws {
    try requireReleaseGate()
    launchReplayWorkload()
    let scroll = app.scrollViews["replay.scroll"]
    XCTAssertTrue(scroll.waitForExistence(timeout: 10))
    let options = singleIterationOptions()
    measure(
      metrics: [XCTCPUMetric(), XCTMemoryMetric(), XCTStorageMetric()],
      options: options
    ) {
      exerciseReplayJourney(for: releaseDuration(defaultSeconds: 3))
    }
  }

  func testNetworkWorkload() throws {
    try requireReleaseGate()
    launchReplayWorkload()
    let webView = app.descendants(matching: .any)["replay.web-view"]
    XCTAssertTrue(webView.waitForExistence(timeout: 10))
    let options = singleIterationOptions()
    measure(metrics: [XCTClockMetric(), XCTStorageMetric()], options: options) {
      app.swipeUp()
      app.swipeDown()
    }
  }

  func testQueuePressureWorkload() throws {
    try requireReleaseGate()
    launchReplayWorkload()
    let burst = app.buttons["queue.emit-burst"]
    XCTAssertTrue(burst.waitForExistence(timeout: 10))
    let options = singleIterationOptions()
    measure(metrics: [XCTCPUMetric(), XCTMemoryMetric(), XCTStorageMetric()], options: options) {
      for _ in 0..<10 {
        burst.tap()
      }
    }
    XCTAssertTrue(app.staticTexts["queue.completed"].exists)
  }

  func testPrivacyCanaryWorkload() throws {
    try requireReleaseGate()
    guard let canary = ProcessInfo.processInfo.environment["CHILL_PRIVACY_CANARY"],
      !canary.isEmpty
    else {
      XCTFail("CHILL_PRIVACY_CANARY is required for the privacy workload")
      return
    }
    app.launchEnvironment["CHILL_PRIVACY_CANARY"] = canary
    app.launch()
    selectTab("Privacy")

    let secureField = app.secureTextFields["privacy.secure-field"]
    XCTAssertTrue(secureField.waitForExistence(timeout: 10))
    secureField.tap()
    secureField.typeText("-edited")
    XCTAssertTrue(app.staticTexts["privacy.text"].exists)
    XCTAssertTrue(app.staticTexts["privacy.error"].exists)
    XCTAssertTrue(app.staticTexts["privacy.network-url"].exists)
  }

  func testReplayJourneyWorkload() throws {
    try requireReleaseGate()
    launchReplayWorkload()
    let options = singleIterationOptions()
    measure(
      metrics: [XCTCPUMetric(), XCTMemoryMetric(), XCTStorageMetric(), XCTClockMetric()],
      options: options
    ) {
      exerciseReplayJourney(for: releaseDuration(defaultSeconds: 3))
    }
  }

  func testAccessibilityActivationWorkload() throws {
    try requireReleaseGate()
    app.launch()
    selectTab("UIKit")

    let button = app.buttons["uikit.button"]
    let toggle = app.switches["uikit.switch"]
    let segmented = app.segmentedControls["uikit.segmented"]
    let slider = app.sliders["uikit.slider"]
    let textField = app.textFields["uikit.text-field"]
    for element in [button, toggle, segmented, slider, textField] {
      XCTAssertTrue(element.waitForExistence(timeout: 10), element.debugDescription)
      XCTAssertTrue(element.isEnabled, element.debugDescription)
    }

    for cycle in 0..<10 {
      button.tap()
      toggle.tap()
      segmented.buttons[cycle.isMultiple(of: 2) ? "Two" : "One"].tap()
      slider.adjust(toNormalizedSliderPosition: cycle.isMultiple(of: 2) ? 0.25 : 0.75)
      textField.tap()
      textField.typeText("\(cycle)")
      XCTAssertEqual(app.state, .runningForeground)
    }
  }

  private func requireReleaseGate() throws {
    try XCTSkipUnless(
      ProcessInfo.processInfo.environment["CHILL_RELEASE_WORKLOAD_GATE"] == "1",
      "Set CHILL_RELEASE_WORKLOAD_GATE=1 only from the physical release runner"
    )
  }

  private func launchReplayWorkload() {
    app.launch()
    selectTab("Replay")
    XCTAssertTrue(app.scrollViews["replay.scroll"].waitForExistence(timeout: 10))
  }

  private func selectTab(_ title: String) {
    let tab = app.tabBars.buttons[title]
    XCTAssertTrue(tab.waitForExistence(timeout: 10))
    tab.tap()
  }

  private func exerciseReplayJourney(for duration: TimeInterval) {
    let deadline = Date().addingTimeInterval(duration)
    repeat {
      app.swipeUp(velocity: .fast)
      app.swipeDown(velocity: .fast)
      let animate = app.buttons["replay.animate"]
      if animate.exists, animate.isHittable {
        animate.tap()
      }
    } while Date() < deadline
  }

  private func releaseDuration(defaultSeconds: TimeInterval) -> TimeInterval {
    guard let raw = ProcessInfo.processInfo.environment["CHILL_RELEASE_DURATION_SECONDS"],
      let value = TimeInterval(raw), value > 0
    else {
      return defaultSeconds
    }
    return value
  }

  private func singleIterationOptions() -> XCTMeasureOptions {
    let options = XCTMeasureOptions()
    options.iterationCount = 1
    return options
  }

  private func attachState(name: String) {
    print("CHILL_HIERARCHY_\(name.uppercased())_BEGIN")
    print(app.debugDescription)
    print("CHILL_HIERARCHY_\(name.uppercased())_END")
    let attachment = XCTAttachment(screenshot: app.screenshot())
    attachment.name = name
    attachment.lifetime = .keepAlways
    add(attachment)
  }
}
