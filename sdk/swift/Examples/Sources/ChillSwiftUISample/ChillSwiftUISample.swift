import Chill
import ChillSampleSupport
import SwiftUI

@main
struct ChillSwiftUISample: App {
  init() {
    try? SampleChill.configure(application: "chill-swiftui-sample")
  }

  var body: some Scene {
    WindowGroup {
      NavigationStack {
        CatListScreen()
      }
      .annotation("account.tier", value: "internal")
      .annotation(["app.area": "adoption", "build.channel": "sample"])
      .page("app", relation: .root)
    }
  }
}

private struct Cat: Identifiable, Hashable {
  let id: String
  let name: String
}

@MainActor
private final class CatRepository {
  @Activity("repository.load_cats", kind: .storage)
  func load() -> [Cat] {
    [
      Cat(id: "cat-123", name: "Juniper"),
      Cat(id: "cat-456", name: "Mochi"),
    ]
  }

  @Event("adoption.requested", emission: .succeeded)
  func requestAdoption(for _: Cat) {}
}

private struct CatListScreen: View {
  @State private var cats: [Cat] = []
  @State private var selected: Cat?
  private let repository = CatRepository()

  var body: some View {
    List(cats) { cat in
      ChillActionButton(
        "cat.open",
        operation: { selected = cat }
      ) {
        HStack {
          Text(cat.name).privacy(.redacted)
          Spacer()
          Image(systemName: "chevron.right")
        }
      }
      .annotation("cat.id", value: cat.id)
      .impression("cat.row", role: "content")
    }
    .navigationTitle("Cats")
    .page("cats")
    .event("cats.loaded", when: cats.count)
    .task { cats = repository.load() }
    .navigationDestination(item: $selected) { cat in
      CatDetailScreen(cat: cat, repository: repository)
    }
  }
}

private struct CatDetailScreen: View {
  let cat: Cat
  let repository: CatRepository

  var body: some View {
    VStack(spacing: 24) {
      Image(systemName: "cat.fill")
        .font(.system(size: 72))
        .replay(.masked)
      Text(cat.name).privacy(.redacted)
      ChillActionButton(
        "Request adoption",
        action: "cat.adopt",
        operation: { repository.requestAdoption(for: cat) }
      )
    }
    .padding()
    .annotation("cat.id", value: cat.id)
    // This loses to the outer declaration by contract.
    .annotation("account.tier", value: "descendant")
    .page("cat")
  }
}
