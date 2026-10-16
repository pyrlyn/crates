import InjectaSyntax
import SwiftParser
import Testing

private func scan(_ files: [String: String]) -> TargetScan {
  var scan = TargetScan()
  for (name, source) in files.sorted(by: { $0.key < $1.key }) {
    scan.add(Parser.parse(source: source), file: name)
  }
  return scan
}

@Test func aMissingEntryOfAnAutoWiredTypeInAnotherFileIsReportedAtTheEntry() {
  let result = scan([
    "Store.swift": """
      @Injectable
      final class SettingsStore {
        init(client: any SettingsClient, cwd: String) {}
      }
      """,
    "Graph.swift": """
      @Container
      final class AppGraph {
        let cwd: String
        @Transient var settings: SettingsStore
      }
      """,
  ])
  let lines = result.diagnostics()
  #expect(lines.count == 1)
  #expect(lines[0].isError)
  #expect(lines[0].line.hasPrefix("Graph.swift:4:"))
  #expect(lines[0].line.contains("'settings' needs 'client'"))
}

@Test func aCycleThroughAutoWiredTypesIsReported() {
  let result = scan([
    "A.swift": "@Injectable struct A { let b: B }\n@Injectable struct B { let a: A }",
    "Graph.swift": "@Container final class G {\n  @Transient var a: A\n  @Transient var b: B\n}",
  ])
  #expect(result.diagnostics().map(\.line).contains { $0.contains("dependency cycle: a -> b -> a") })
}

@Test func problemsTheMacroAlreadyReportsAreNotRepeated() {
  let result = scan([
    "Graph.swift": """
      @Container final class G {
        @Provides func makeA(missing: Int) -> Int { missing }
      }
      """
  ])
  #expect(result.diagnostics().isEmpty)
}

@Test func aSoundTargetHasNoDiagnostics() {
  let result = scan([
    "All.swift": """
      @Injectable struct Repo { let db: Database }
      @Injectable final class Database { init(config: Config) {} }
      @Container final class G {
        let config: Config
        @Singleton var db: Database
        @Transient var repo: Repo
      }
      """
  ])
  #expect(result.diagnostics().isEmpty)
  #expect(result.needs(of: "Database")?.edges == ["config"])
  #expect(result.needs(of: "Database")?.deferred.isEmpty == true)
  #expect(result.needs(of: "App.Repo")?.edges == ["db"])
}

@Test func aLazyOrFactoryDependencyIsNotACycleOrACaptive() {
  let result = scan([
    "All.swift": """
      @Injectable final class Clock { init(counter: Counter) {} }
      @Injectable final class Cache { init(clock: () -> Clock) {} }
      @Injectable final class Left { init(right: Lazy<Right>) {} }
      @Injectable final class Right { init(left: Left) {} }
      @Container final class G {
        let counter: Counter
        @Transient var clock: Clock
        @Singleton var cache: Cache
        @Singleton var left: Left
        @Singleton var right: Right
      }
      """
  ])
  #expect(result.diagnostics().isEmpty)
  #expect(result.needs(of: "Cache")?.deferred == ["clock"])
  #expect(result.needs(of: "Left")?.deferred == ["right"])
  #expect(result.needs(of: "Right")?.edges == ["left"])
}

@Test func aDirectSingletonOfATransientIsStillAWarning() {
  let result = scan([
    "All.swift": """
      @Injectable struct Clock {}
      @Injectable final class Cache { init(clock: Clock) {} }
      @Container final class G {
        @Transient var clock: Clock
        @Singleton var cache: Cache
      }
      """
  ])
  #expect(result.diagnostics().map(\.line).contains { $0.contains("captures transient 'clock'") })
}

@Test func conformancesAreGeneratedOncePerAutoWiredTypeWithTheContainersIsolation() {
  let result = scan([
    "Graph.swift": """
      @MainActor @Container final class UI {
        @Singleton var settings: SettingsStore
        @Transient var other: SettingsStore
        @Provides func makeCore() -> any Core { LiveCore() }
      }
      @Container final class Plain {
        @Transient var repo: Repo
      }
      """
  ])
  #expect(result.conformances() == [
    "extension UI: @MainActor SettingsStore.Needs {}",
    "extension Plain: Repo.Needs {}",
  ])
}
