import SwiftUI
import UIKit
import WebKit

#if CHILL_CANDIDATE
  import Chill
  import ChillReplay
#endif

@main
struct ChillPhysicalValidationApp: App {
  init() {
    UIDevice.current.isBatteryMonitoringEnabled = true
    configureCandidateRuntime()
  }

  var body: some Scene {
    WindowGroup {
      ValidationRootView()
        #if CHILL_CANDIDATE
          .annotation("validation.scope", value: "physical")
          .page("validation", relation: .root)
        #endif
    }
  }

  private func configureCandidateRuntime() {
    #if CHILL_CANDIDATE
      let runtime = ChillRuntime(
        configuration: try! RuntimeConfiguration(
          sessionID: SessionID("physical-validation-session"),
          bootID: BootID("physical-validation-boot"),
          privacy: PrivacyPolicy(
            version: "physical-validation-v1",
            analytics: .granted,
            diagnostic: .granted,
            replay: .granted,
            annotationAllowlist: [
              "cat.id": .pseudonymousIdentifier,
              "validation.scope": .internalData,
            ]
          ),
          sampling: SamplingConfiguration(
            stableKey: "physical-validation-device",
            salt: "physical-validation-v1",
            replay: .all
          )
        ),
        sink: NoOpRecordSink()
      )
      Chill.configure(runtime)
      let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
        "chill-physical-replay",
        isDirectory: true
      )
      try? ChillSessionReplay.configure(
        .standard(directory: directory, keyIdentifier: "physical-validation")
      )
    #endif
  }
}

private enum ValidationTab: Hashable {
  case journey
  case workload
  case privacy
  case uikit
}

private struct ValidationRootView: View {
  @State private var selection: ValidationTab = .journey

  var body: some View {
    TabView(selection: $selection) {
      NavigationStack {
        ValidationList()
      }
      .tabItem { Label("Journey", systemImage: "list.bullet") }
      .tag(ValidationTab.journey)
      .accessibilityIdentifier("tab.journey")

      ReplayWorkloadView()
        .tabItem { Label("Replay", systemImage: "record.circle") }
        .tag(ValidationTab.workload)
        .accessibilityIdentifier("tab.replay")

      PrivacyCanaryView()
        .tabItem { Label("Privacy", systemImage: "hand.raised") }
        .tag(ValidationTab.privacy)
        .accessibilityIdentifier("tab.privacy")

      UIKitValidationView()
        .tabItem { Label("UIKit", systemImage: "switch.2") }
        .tag(ValidationTab.uikit)
        .accessibilityIdentifier("tab.uikit")
    }
    .overlay(alignment: .topTrailing) {
      ReleaseEnvironmentBadge()
        .padding(8)
    }
  }
}

private struct ReleaseEnvironmentBadge: View {
  private var value: String {
    let process = ProcessInfo.processInfo
    return [
      "physical=\(!process.environment.keys.contains("SIMULATOR_DEVICE_NAME"))",
      "os=\(UIDevice.current.systemVersion)",
      "thermal=\(process.thermalState.validationName)",
      "power=\(UIDevice.current.batteryState.validationName)",
    ].joined(separator: ";")
  }

  var body: some View {
    Text("Release environment")
      .font(.caption2)
      .padding(4)
      .background(.thinMaterial, in: Capsule())
      .accessibilityIdentifier("validation.release-environment")
      .accessibilityValue(value)
  }
}

private struct ValidationCat: Identifiable, Hashable {
  let id: String
  let name: String
}

private struct ValidationList: View {
  private let cats = [
    ValidationCat(id: "cat-123", name: "Juniper"),
    ValidationCat(id: "cat-456", name: "Mochi"),
  ]
  @State private var selected: ValidationCat?
  @State private var query = ""
  @State private var notifications = true

  var body: some View {
    List {
      Section("Journey state") {
        TextField("Search", text: $query)
          .textInputAutocapitalization(.never)
          .accessibilityIdentifier("journey.search")
        Toggle("Notifications", isOn: $notifications)
          .accessibilityIdentifier("validation.notifications")
      }

      Section("Cats") {
        ForEach(cats) { cat in
          candidateRow(cat)
        }
      }
    }
    .navigationTitle("Physical validation")
    .accessibilityIdentifier("validation.list")
    #if CHILL_CANDIDATE
      .page("cats")
      .event("validation.loaded", when: cats.count)
    #endif
    .navigationDestination(item: $selected) { cat in
      ValidationDetail(cat: cat)
    }
  }

  @ViewBuilder
  private func candidateRow(_ cat: ValidationCat) -> some View {
    #if CHILL_CANDIDATE
      ChillActionButton(
        cat.name,
        action: "cat.open",
        operation: { selected = cat }
      )
      .annotation("cat.id", value: cat.id)
      .impression("cat.row", role: "content")
      .accessibilityIdentifier("cat.\(cat.id)")
    #else
      Button(cat.name) { selected = cat }
        .accessibilityIdentifier("cat.\(cat.id)")
    #endif
  }
}

private struct ValidationDetail: View {
  let cat: ValidationCat

  var body: some View {
    VStack(spacing: 24) {
      Image(systemName: "cat.fill")
        .font(.system(size: 72))
        .accessibilityHidden(true)
      Text(cat.name)
        .privacySensitive()
      candidateAdoptionButton
    }
    .padding()
    .navigationTitle("Cat")
    .accessibilityIdentifier("validation.detail")
    #if CHILL_CANDIDATE
      .annotation("cat.id", value: cat.id)
      .page("cat")
      .replay(.masked)
    #endif
  }

  @ViewBuilder
  private var candidateAdoptionButton: some View {
    #if CHILL_CANDIDATE
      ChillActionButton(
        "Request adoption",
        action: "cat.adopt",
        operation: {}
      )
      .accessibilityIdentifier("cat.adopt")
    #else
      Button("Request adoption") {}
        .accessibilityIdentifier("cat.adopt")
    #endif
  }
}

private struct ReplayRow: Identifiable {
  let id: Int
}

private struct ReplayWorkloadView: View {
  private let rows = (0..<240).map(ReplayRow.init(id:))
  @State private var showSheet = false
  @State private var animate = false
  @State private var formValue = ""
  @State private var emittedEvents = 0

  var body: some View {
    NavigationStack {
      ScrollView {
        VStack(spacing: 16) {
          controls
          replayCanvas
          ValidationWebView()
            .frame(height: 140)
            .accessibilityIdentifier("replay.web-view")
          LazyVStack {
            ForEach(rows) { row in
              Text("Replay row \(row.id)")
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 6)
                .accessibilityIdentifier("replay.row.\(row.id)")
            }
          }
        }
        .padding()
      }
      .accessibilityIdentifier("replay.scroll")
      .navigationTitle("Replay workload")
      .sheet(isPresented: $showSheet) {
        NavigationStack {
          Form {
            TextField("Form value", text: $formValue)
              .accessibilityIdentifier("replay.form-field")
          }
          .navigationTitle("Replay sheet")
          .toolbar {
            Button("Done") { showSheet = false }
              .accessibilityIdentifier("replay.sheet.done")
          }
        }
      }
    }
  }

  private var controls: some View {
    VStack(spacing: 12) {
      Button("Animate") {
        withAnimation(.easeInOut(duration: 0.2)) { animate.toggle() }
      }
      .accessibilityIdentifier("replay.animate")
      Button("Present form") { showSheet = true }
        .accessibilityIdentifier("replay.present-form")
      Button("Emit queue burst") {
        for sequence in emittedEvents..<(emittedEvents + 1_000) {
          emitValidationWorkloadEvent(sequence)
        }
        emittedEvents += 1_000
      }
      .accessibilityIdentifier("queue.emit-burst")
      Text("events=\(emittedEvents)")
        .accessibilityIdentifier("queue.completed")
    }
  }

  private var replayCanvas: some View {
    Canvas { context, size in
      let rect = CGRect(origin: .zero, size: size).insetBy(dx: 8, dy: 8)
      context.fill(
        Path(roundedRect: rect, cornerRadius: animate ? 36 : 8),
        with: .color(animate ? .orange : .blue)
      )
    }
    .frame(height: 100)
    .accessibilityLabel("Replay custom drawing")
    .accessibilityIdentifier("replay.canvas")
  }
}

private struct PrivacyCanaryView: View {
  @State private var editableCanary = ""

  private var canary: String {
    ProcessInfo.processInfo.environment["CHILL_PRIVACY_CANARY"] ?? "canary-not-provided"
  }

  var body: some View {
    NavigationStack {
      Form {
        Section("Visible surfaces") {
          Text(canary)
            .privacySensitive()
            .accessibilityIdentifier("privacy.text")
          SecureField("Secure canary", text: $editableCanary)
            .accessibilityIdentifier("privacy.secure-field")
          Text("Accessibility canary")
            .accessibilityLabel(canary)
            .accessibilityIdentifier("privacy.accessibility-label")
          Text("Error: \(canary)")
            .foregroundStyle(.red)
            .accessibilityIdentifier("privacy.error")
        }
        Section("Network metadata") {
          Text("https://127.0.0.1/validation?token=\(canary)")
            .privacySensitive()
            .accessibilityIdentifier("privacy.network-url")
        }
      }
      .navigationTitle("Privacy canaries")
      .onAppear { editableCanary = canary }
      #if CHILL_CANDIDATE
        .replay(.masked)
      #endif
    }
  }
}

private struct ValidationWebView: UIViewRepresentable {
  func makeUIView(context _: Context) -> WKWebView {
    let webView = WKWebView(frame: .zero)
    webView.isAccessibilityElement = true
    webView.accessibilityLabel = "Static validation web content"
    webView.loadHTMLString(
      "<html><body><h1>Replay web view</h1><input type='text' value='masked'></body></html>",
      baseURL: nil
    )
    return webView
  }

  func updateUIView(_: WKWebView, context _: Context) {}
}

private struct UIKitValidationView: UIViewControllerRepresentable {
  func makeUIViewController(context _: Context) -> ValidationUIKitViewController {
    ValidationUIKitViewController()
  }

  func updateUIViewController(_: ValidationUIKitViewController, context _: Context) {}
}

private final class ValidationUIKitViewController: UIViewController {
  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .systemBackground

    let stack = UIStackView(arrangedSubviews: [
      makeButton(),
      makeSwitch(),
      makeSegmentedControl(),
      makeSlider(),
      makeTextField(),
    ])
    stack.axis = .vertical
    stack.spacing = 28
    stack.translatesAutoresizingMaskIntoConstraints = false
    view.addSubview(stack)
    NSLayoutConstraint.activate([
      stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
      stack.trailingAnchor.constraint(
        equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),
      stack.centerYAnchor.constraint(equalTo: view.safeAreaLayoutGuide.centerYAnchor),
    ])
  }

  private func makeButton() -> UIButton {
    #if CHILL_CANDIDATE
      let control = ChillButton(type: .system).action("validation.uikit.button")
    #else
      let control = UIButton(type: .system)
    #endif
    control.setTitle("UIKit button", for: .normal)
    control.accessibilityIdentifier = "uikit.button"
    return control
  }

  private func makeSwitch() -> UISwitch {
    #if CHILL_CANDIDATE
      let control = ChillSwitch(frame: .zero).action("validation.uikit.switch")
    #else
      let control = UISwitch(frame: .zero)
    #endif
    control.accessibilityLabel = "UIKit switch"
    control.accessibilityIdentifier = "uikit.switch"
    return control
  }

  private func makeSegmentedControl() -> UISegmentedControl {
    #if CHILL_CANDIDATE
      let control = ChillSegmentedControl(items: ["One", "Two"])
        .action("validation.uikit.segmented")
    #else
      let control = UISegmentedControl(items: ["One", "Two"])
    #endif
    control.selectedSegmentIndex = 0
    control.accessibilityIdentifier = "uikit.segmented"
    return control
  }

  private func makeSlider() -> UISlider {
    #if CHILL_CANDIDATE
      let control = ChillSlider(frame: .zero).action("validation.uikit.slider")
    #else
      let control = UISlider(frame: .zero)
    #endif
    control.value = 0.5
    control.accessibilityLabel = "UIKit slider"
    control.accessibilityIdentifier = "uikit.slider"
    return control
  }

  private func makeTextField() -> UITextField {
    #if CHILL_CANDIDATE
      let control = ChillTextField(frame: .zero).action("validation.uikit.text-field")
    #else
      let control = UITextField(frame: .zero)
    #endif
    control.borderStyle = .roundedRect
    control.placeholder = "UIKit text field"
    control.accessibilityIdentifier = "uikit.text-field"
    return control
  }
}

#if CHILL_CANDIDATE
  @Event("validation.queue.event")
  private func emitValidationWorkloadEvent(_: Int) {}
#else
  private func emitValidationWorkloadEvent(_: Int) {}
#endif

extension ProcessInfo.ThermalState {
  fileprivate var validationName: String {
    switch self {
    case .nominal: "nominal"
    case .fair: "fair"
    case .serious: "serious"
    case .critical: "critical"
    @unknown default: "unknown"
    }
  }
}

extension UIDevice.BatteryState {
  fileprivate var validationName: String {
    switch self {
    case .unknown: "unknown"
    case .unplugged: "unplugged"
    case .charging: "charging"
    case .full: "full"
    @unknown default: "unknown"
    }
  }
}
