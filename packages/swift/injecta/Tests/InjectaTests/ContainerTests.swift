import Injecta
import Synchronization
import Testing

private func app(_ overrides: AppGraph.Overrides = .init()) -> AppGraph {
  AppGraph(config: Config(poolSize: 4), counter: Counter(), overrides: overrides)
}

@Suite("Resolution")
struct ResolutionTests {
  @Test func autoWiredTypesReceiveTheirDependenciesThroughTheInjectedInitializer() {
    let graph = app()
    let handler = graph.handler
    #expect(handler.tag == "default")
    #expect(handler.service.retries == 3)
    #expect(handler.service.repo.db.config == Config(poolSize: 4))
    #expect(handler.service.logger.level == 4)
  }

  @Test func aProviderReceivesTheEntriesNamedByItsLabels() {
    #expect(app().logger.level == 4)
  }

  @Test func theGraphHasNoIssues() {
    #expect(AppGraph.injectaGraph.issues().isEmpty)
    #expect(RequestGraph.injectaGraph.issues().isEmpty)
    #expect(UIGraph.injectaGraph.issues().isEmpty)
  }

  @Test func theGraphListsAutoWiredNeedsFromTheInjectableTypes() {
    let graph = AppGraph.injectaGraph
    #expect(graph.node(named: "db")?.dependencies == ["config", "logger", "counter"])
    #expect(graph.node(named: "service")?.dependencies == ["repo", "logger"])
    #expect(graph.node(named: "handler")?.dependencies == ["service"])
    #expect(graph.node(named: "logger")?.dependencies == ["config"])
    #expect(graph.node(named: "config")?.lifetime == .input)
  }
}

@Suite("Lifetimes")
struct LifetimeTests {
  @Test func aSingletonIsBuiltOnceAndShared() {
    let graph = app()
    let first = graph.db
    let second = graph.db
    #expect(first === second)
    #expect(graph.counter.count == 1)
  }

  @Test func aSingletonIsBuiltOnFirstUseNotAtInit() {
    let graph = app()
    #expect(graph.counter.count == 0)
    _ = graph.repo
    #expect(graph.counter.count == 1)
  }

  @Test func anEagerSingletonIsBuiltInInit() {
    let graph = EagerGraph(config: Config(poolSize: 1), counter: Counter())
    #expect(graph.counter.count == 1)
    _ = graph.db
    #expect(graph.counter.count == 1)
  }

  @Test func aTransientIsNewOnEveryAccessButSharesSingletons() {
    let graph = app()
    let first = graph.handler
    let second = graph.handler
    #expect(first !== second)
    #expect(first.service.repo.db === second.service.repo.db)
  }

  @Test func eachContainerInstanceHasItsOwnSingletons() {
    #expect(app().db !== app().db)
  }

  @Test func concurrentFirstAccessBuildsTheSingletonExactlyOnce() async {
    let graph = app()
    let ids = await withTaskGroup(of: ObjectIdentifier.self) { group in
      for _ in 0..<64 { group.addTask { ObjectIdentifier(graph.db) } }
      return await group.reduce(into: Set<ObjectIdentifier>()) { $0.insert($1) }
    }
    #expect(ids.count == 1)
    #expect(graph.counter.count == 1)
  }
}

@Suite("Scopes")
struct ScopeTests {
  @Test func aChildScopeForwardsTheParentsSingletons() {
    let root = app()
    let request = RequestGraph(parent: root, requestID: 7)
    #expect(request.db === root.db)
    #expect(request.logger.level == root.logger.level)
  }

  @Test func aScopedSingletonIsSharedInsideOneScopeOnly() {
    let root = app()
    let one = RequestGraph(parent: root, requestID: 1)
    let two = RequestGraph(parent: root, requestID: 2)
    #expect(one.repo.db === two.repo.db)
    #expect(one.trace == "req-1")
    #expect(two.trace == "req-2")
  }
}

@Suite("Overrides")
struct OverrideTests {
  @Test func aSingletonOverrideIsTheValueFromTheStart() {
    var overrides = AppGraph.Overrides()
    overrides.logger = SilentLogger()
    let graph = app(overrides)
    #expect(graph.logger.level == 0)
    #expect(graph.handler.service.logger.level == 0)
  }

  @Test func aTransientOverrideReplacesItsProvider() {
    let fake = UserService(repo: UserRepo(db: app().db, logger: SilentLogger()), logger: SilentLogger(), retries: 9)
    var overrides = AppGraph.Overrides()
    overrides.service = { fake }
    #expect(app(overrides).handler.service.retries == 9)
  }

  @Test func anOverriddenSingletonIsNeverBuilt() {
    let counter = Counter()
    var overrides = AppGraph.Overrides()
    overrides.db = Database(config: Config(poolSize: 99), logger: SilentLogger(), counter: counter)
    let graph = AppGraph(config: Config(poolSize: 1), counter: Counter(), overrides: overrides)
    #expect(graph.db.config.poolSize == 99)
    #expect(graph.counter.count == 0)
  }
}

@MainActor
@Suite("Main actor")
struct MainActorTests {
  @Test func aMainActorContainerBuildsMainActorTypes() {
    let graph = UIGraph(logger: ConsoleLogger(level: 2))
    #expect(graph.settings === graph.settings)
    #expect(graph.settings.logger.level == 2)
  }

  @Test func aPreviewOverridesTheModel() {
    var overrides = UIGraph.Overrides()
    let preview = SettingsModel(logger: SilentLogger())
    preview.title = "Preview"
    overrides.settings = preview
    #expect(UIGraph(logger: SilentLogger(), overrides: overrides).settings.title == "Preview")
  }
}

@Suite("Deferred dependencies")
struct DeferredTests {
  @Test func aFactoryMakesANewTransientOnEveryCall() {
    let counter = Counter()
    let graph = FactoryGraph(counter: counter)
    let cache = graph.cache
    #expect(counter.count == 0)
    let first = cache.clock()
    let second = cache.clock()
    #expect(first !== second)
    #expect(first.id == 1)
    #expect(second.id == 2)
  }

  @Test func aProviderFactoryIsGeneratedAsAClosure() {
    let graph = ProviderFactoryGraph(counter: Counter())
    #expect(graph.cache.clock() !== graph.cache.clock())
  }

  @Test func lazyBreaksACycleAndBuildsThePeerOnce() {
    let graph = FactoryGraph(counter: Counter())
    let left = graph.left
    let right = left.right.value
    #expect(right.left === left)
    #expect(left.right.value === right)
    #expect(graph.right === right)
  }

  @Test func deferredNeedsAreListedApartFromEdgesAndAreNotIssues() {
    let graph = FactoryGraph.injectaGraph
    #expect(graph.issues().isEmpty)
    #expect(graph.node(named: "cache")?.dependencies.isEmpty == true)
    #expect(graph.node(named: "cache")?.deferred == ["clock"])
    #expect(graph.node(named: "left")?.deferred == ["right"])
    #expect(graph.node(named: "right")?.dependencies == ["left"])
    #expect(graph.describe().contains("cache: Cache [singleton] <- ~clock"))
  }
}

@Suite("Introspection")
struct IntrospectionTests {
  @Test func describeListsEveryEntry() {
    let text = AppGraph.injectaGraph.describe()
    #expect(text.contains("db: Database [singleton] <- config, logger, counter"))
    #expect(text.contains("handler: Handler [transient] <- service"))
  }

  @Test func dotExportsEdges() {
    #expect(AppGraph.injectaGraph.dot().contains("\"handler\" -> \"service\";"))
  }
}

@Suite("Once")
struct OnceTests {
  // `#expect` copies its operands, and `Once` is noncopyable, so results are read first.
  @Test func seedWinsOverTheBuilder() {
    let once = Once<Int>()
    once.seed(5)
    let value = once.get { 9 }
    #expect(value == 5)
  }

  @Test func seedAfterBuildIsIgnored() {
    let once = Once<Int>()
    let first = once.get { 1 }
    once.seed(2)
    let second = once.get { 3 }
    let built = once.isBuilt
    #expect(first == 1)
    #expect(second == 1)
    #expect(built)
  }

  @Test func theValueIsReleasedWithTheCell() {
    final class Probe {}
    weak var weakProbe: Probe?
    do {
      let once = Once<Probe>()
      let probe = once.get { Probe() }
      weakProbe = probe
    }
    #expect(weakProbe == nil)
  }
}
