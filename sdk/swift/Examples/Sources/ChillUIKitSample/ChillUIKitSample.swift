#if os(iOS)
  import Chill
  import ChillSampleSupport
  import UIKit

  @main
  @MainActor
  final class AppDelegate: UIResponder, UIApplicationDelegate {
    var window: UIWindow?

    func application(
      _: UIApplication,
      didFinishLaunchingWithOptions _: [UIApplication.LaunchOptionsKey: Any]?
    ) -> Bool {
      try? SampleChill.configure(application: "chill-uikit-sample")

      let root = UINavigationController(
        rootViewController: CatsViewController()
      )
      .annotation("account.tier", value: "internal")
      .annotation(["app.area": "adoption", "build.channel": "sample"])
      .page("app", relation: .root)
      let window = UIWindow(frame: UIScreen.main.bounds)
      window.rootViewController = root
      window.makeKeyAndVisible()
      self.window = window
      return true
    }
  }

  @MainActor
  private final class CatsViewController: UIViewController {
    private let repository = CatRepository()

    init() {
      super.init(nibName: nil, bundle: nil)
      page("cats")
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) { nil }

    override func viewDidLoad() {
      super.viewDidLoad()
      view.backgroundColor = .systemBackground
      title = "Cats"

      let button = ChillButton(type: .system)
        .annotation("cat.id", value: "cat-123")
        .action("cat.open")
      button.setTitle("Open Juniper", for: .normal)
      button.accessibilityHint = "Shows adoption details"
      button.addAction(
        UIAction { [weak self] _ in self?.openCat() },
        for: .primaryActionTriggered
      )
      button.translatesAutoresizingMaskIntoConstraints = false
      view.addSubview(button)
      NSLayoutConstraint.activate([
        button.centerXAnchor.constraint(equalTo: view.centerXAnchor),
        button.centerYAnchor.constraint(equalTo: view.centerYAnchor),
      ])
    }

    @Activity("repository.load_cat", kind: .storage)
    private func openCat() {
      let cat = repository.load()
      navigationController?.pushViewController(
        CatViewController(cat: cat, repository: repository),
        animated: true
      )
    }
  }

  private struct Cat: Sendable {
    let id: String
    let name: String
  }

  private final class CatRepository: Sendable {
    func load() -> Cat { Cat(id: "cat-123", name: "Juniper") }

    @Event("adoption.requested", emission: .succeeded)
    func requestAdoption(for _: Cat) {}
  }

  @MainActor
  private final class CatViewController: UIViewController {
    private let cat: Cat
    private let repository: CatRepository

    init(cat: Cat, repository: CatRepository) {
      self.cat = cat
      self.repository = repository
      super.init(nibName: nil, bundle: nil)
      annotation("cat.id", value: cat.id)
      // This loses to the navigation controller's outer declaration.
      annotation("account.tier", value: "descendant")
      page("cat")
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) { nil }

    override func viewDidLoad() {
      super.viewDidLoad()
      view.backgroundColor = .systemBackground
      title = cat.name

      let button = ChillButton(type: .system)
        .action("cat.adopt")
      button.setTitle("Request adoption", for: .normal)
      button.addAction(
        UIAction { [repository, cat] _ in
          repository.requestAdoption(for: cat)
        },
        for: .primaryActionTriggered
      )
      button.translatesAutoresizingMaskIntoConstraints = false
      view.addSubview(button)
      NSLayoutConstraint.activate([
        button.centerXAnchor.constraint(equalTo: view.centerXAnchor),
        button.centerYAnchor.constraint(equalTo: view.centerYAnchor),
      ])
    }
  }
#else
  import Foundation

  @main
  enum ChillUIKitSampleUnavailable {
    static func main() {
      print("ChillUIKitSample is available when built for iOS.")
    }
  }
#endif
