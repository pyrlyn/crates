import InjectaMacros
import SwiftSyntaxMacros
import SwiftSyntaxMacrosTestSupport
import XCTest

final class ContainerMacroTests: XCTestCase {
  func testAContainerExpandsToStaticCallsWithoutLookups() {
    assertExpansion(
      """
      @Container
      final class AppGraph {
        let home: String
        @Provides(.singleton) func makeCore(home: String) -> any CoreClient { LiveCore(home: home) }
        @Transient var settings: SettingsStore
      }
      """,
      """
        final class AppGraph {
          let home: String
          func makeCore(home: String) -> any CoreClient { LiveCore(home: home) }

          private let _injecta_core = Injecta.Once<any CoreClient>()

          var core: any CoreClient {
            _injecta_core.get {
              self.makeCore(home: self.home)
            }
          }
          var settings: SettingsStore {
            get {
              if let make = _injectaOverrides.settings {
                return make()
              }
              #if DEBUG
              guard let needs = self as? SettingsStore.Needs else {
                Injecta.missingPlugin("SettingsStore")
              }
              return SettingsStore(injecting: needs)
              #else
              return SettingsStore(injecting: self)
              #endif
            }
          }

          /// Replacements for tests and previews. A singleton set here is the value from the start
          /// (no cost on the resolve path); a transient set here is called instead of its provider.
          struct Overrides: Sendable {
            var core: (any CoreClient)? = nil
            var settings: (@Sendable () -> SettingsStore)? = nil
            init() {
            }
          }

          private let _injectaOverrides: Overrides

          init(home: String, overrides: Overrides = Overrides()) {
            self.home = home
            self._injectaOverrides = overrides
            if let value = overrides.core {
              _injecta_core.seed(value)
            }
            #if DEBUG
            if !(self is SettingsStore.Needs) {
              Injecta.missingPlugin("SettingsStore")
            }
            #endif
          }

          /// Every entry and what it needs; check it in a test with `injectaGraph.issues()`.
          static let injectaGraph = Injecta.Graph(
            container: "AppGraph",
            nodes: [
              Injecta.Node(name: "home", lifetime: .input, type: "String", dependencies: [], deferred: []),
              Injecta.Node(name: "core", lifetime: .singleton, type: "any CoreClient", dependencies: ["home"], deferred: []),
              Injecta.Node(name: "settings", lifetime: .transient, type: "SettingsStore", dependencies: SettingsStore.injectaDependencies, deferred: SettingsStore.injectaDeferred),
            ])
        }

        extension AppGraph: Injecta.Container {
        }
        """)
  }

  func testAMissingProviderLabelIsAnErrorAtTheParameter() {
    assertMacroExpansion(
      """
      @Container
      final class G {
        @Provides func makeCore(home: String) -> Int { 1 }
      }
      """,
      expandedSource: expand(
        """
        @Container
        final class G {
          @Provides func makeCore(home: String) -> Int { 1 }
        }
        """),
      diagnostics: [
        DiagnosticSpec(
          message: "'core' needs 'home', but the container has no entry named 'home'. Add an init input `let home: <Type>`, a provider `@Provides(.singleton) func makeHome(...) -> <Type>`, or, for an @Injectable type, `@Singleton var home: <Type>`.",
          line: 3, column: 27)
      ],
      macroSpecs: specs, indentationWidth: .spaces(2))
  }

  func testALazyProviderParameterIsNotACycle() {
    let diagnostics = expandDiagnostics(
      """
      @Container
      final class G {
        @Provides(.singleton) func makeA(b: Lazy<Int>) -> Int { b.value }
        @Provides(.singleton) func makeB(a: Int) -> Int { a }
      }
      """)
    XCTAssertTrue(diagnostics.isEmpty, "\(diagnostics)")
  }

  func testAFactoryProviderIsPassedAClosure() {
    let expanded = expand(
      """
      @Container
      final class G {
        @Transient var clock: Clock
        @Provides(.singleton) func makeCache(clock: @escaping @Sendable () -> Clock) -> Cache { Cache(clock: clock) }
      }
      """)
    let squeezed = expanded.filter { !$0.isWhitespace }
    XCTAssertTrue(squeezed.contains("self.makeCache(clock:{@Sendableinself.clock})"), expanded)
    XCTAssertTrue(expanded.contains("deferred: [\"clock\"]"), expanded)
    XCTAssertFalse(expanded.contains("dependency cycle"), expanded)
  }

  func testACycleBetweenProvidersIsAnError() {
    let diagnostics = expandDiagnostics(
      """
      @Container
      final class G {
        @Provides(.transient) func makeA(b: Int) -> Int { b }
        @Provides(.transient) func makeB(a: Int) -> Int { a }
      }
      """)
    XCTAssertTrue(diagnostics.contains { $0.contains("dependency cycle: a -> b -> a") }, "\(diagnostics)")
  }

  func testAnEntryWithAProtocolTypeMustUseAProvider() {
    let diagnostics = expandDiagnostics(
      """
      @Container
      final class G {
        @Singleton var core: any CoreClient
      }
      """)
    XCTAssertTrue(
      diagnostics.contains { $0.contains("'any CoreClient' cannot be auto-wired") && $0.contains("func makeCore(...)") },
      "\(diagnostics)")
  }

  func testSingletonOnAFunctionPointsToProvides() {
    let diagnostics = expandDiagnostics(
      """
      @Container
      final class G {
        @Singleton func makeCore() -> Int { 1 }
      }
      """)
    XCTAssertTrue(diagnostics.contains { $0.contains("`@Provides(.singleton)`") }, "\(diagnostics)")
  }

  func testAnEagerProviderIsBuiltInInit() {
    let expanded = expand(
      """
      @Container
      final class G {
        @Provides(.eagerSingleton) func makeClock() -> Clock { Clock() }
      }
      """)
    XCTAssertTrue(expanded.contains("_ = self.clock"), expanded)
  }

  func testAStructOrNonFinalClassIsRejected() {
    XCTAssertTrue(
      expandDiagnostics("@Container struct G {}").contains { $0.contains("`final class`") })
    XCTAssertTrue(
      expandDiagnostics("@Container class G {}").contains { $0.contains("must be `final`") })
  }

  func testForwardNeedsAParentInput() {
    let diagnostics = expandDiagnostics(
      """
      @Container
      final class G {
        @Forward var db: Database
      }
      """)
    XCTAssertEqual(diagnostics.filter { $0.contains("parent") }.count, 1, "\(diagnostics)")
  }

  func testAMainActorContainerUsesIsolatedConformancesAndNonSendableOverrides() {
    let expanded = expand(
      """
      @MainActor @Container
      final class UI {
        @Transient var settings: SettingsStore
      }
      """)
    XCTAssertTrue(expanded.contains("struct Overrides {"), expanded)
    XCTAssertTrue(expanded.contains("var settings: (() -> SettingsStore)? = nil"), expanded)
    XCTAssertTrue(expanded.contains("nonisolated static let injectaGraph"), expanded)
  }
}
