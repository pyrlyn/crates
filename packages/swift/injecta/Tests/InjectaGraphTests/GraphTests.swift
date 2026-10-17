import InjectaGraph
import Testing

@Test func aSoundGraphHasNoIssues() {
  let graph = Graph(container: "App", nodes: [
    Node(name: "config", lifetime: .input, type: "Config"),
    Node(name: "db", lifetime: .singleton, type: "Database", dependencies: ["config"]),
    Node(name: "repo", lifetime: .transient, type: "Repo", dependencies: ["db"]),
  ])
  #expect(graph.issues().isEmpty)
}

@Test func aMissingEntryIsReportedWithAFix() {
  let graph = Graph(container: "App", nodes: [
    Node(name: "db", lifetime: .singleton, type: "Database", dependencies: ["config"])
  ])
  #expect(graph.issues() == [.missing(node: "db", dependency: "config")])
  #expect(graph.issues()[0].description.contains("let config: <Type>"))
  #expect(graph.issues()[0].isError)
}

@Test func aCycleIsReportedOnceInAStableOrder() {
  let graph = Graph(container: "App", nodes: [
    Node(name: "b", lifetime: .transient, type: "B", dependencies: ["c"]),
    Node(name: "a", lifetime: .transient, type: "A", dependencies: ["b"]),
    Node(name: "c", lifetime: .transient, type: "C", dependencies: ["a"]),
  ])
  #expect(graph.issues() == [.cycle(["a", "b", "c", "a"])])
}

@Test func aSelfDependencyIsACycle() {
  let graph = Graph(container: "App", nodes: [
    Node(name: "a", lifetime: .singleton, type: "A", dependencies: ["a"])
  ])
  #expect(graph.issues() == [.cycle(["a", "a"])])
}

@Test func aSingletonHoldingATransientIsAWarning() {
  let graph = Graph(container: "App", nodes: [
    Node(name: "clock", lifetime: .transient, type: "Clock"),
    Node(name: "cache", lifetime: .singleton, type: "Cache", dependencies: ["clock"]),
  ])
  let issues = graph.issues()
  #expect(issues == [.captive(singleton: "cache", transient: "clock")])
  #expect(!issues[0].isError)
}

@Test func aDeferredDependencyIsNotAnEdge() {
  let graph = Graph(container: "App", nodes: [
    Node(name: "clock", lifetime: .transient, type: "Clock"),
    Node(name: "cache", lifetime: .singleton, type: "Cache", deferred: ["clock"]),
    Node(name: "left", lifetime: .singleton, type: "Left", deferred: ["right"]),
    Node(name: "right", lifetime: .singleton, type: "Right", dependencies: ["left"]),
  ])
  #expect(graph.issues().isEmpty)
  #expect(graph.describe().contains("cache: Cache [singleton] <- ~clock"))
  #expect(graph.describe().contains("left: Left [singleton] <- ~right"))
  #expect(graph.dot().contains("\"cache\" -> \"clock\" [style=dashed];"))
  #expect(!graph.dot().contains("\"cache\" -> \"clock\";"))
}

@Test func aMissingDeferredEntryIsStillReported() {
  let graph = Graph(container: "App", nodes: [
    Node(name: "cache", lifetime: .singleton, type: "Cache", deferred: ["clock"])
  ])
  #expect(graph.issues() == [.missing(node: "cache", dependency: "clock")])
}

@Test func describeAndDotNameEveryEntry() {
  let graph = Graph(container: "App", nodes: [
    Node(name: "db", lifetime: .singleton, type: "Database", dependencies: ["config"]),
    Node(name: "config", lifetime: .input, type: "Config"),
  ])
  #expect(graph.describe() == "db: Database [singleton] <- config\nconfig: Config [input]")
  #expect(graph.dot().contains("\"db\" [shape=box, label=\"db: Database\"];"))
  #expect(graph.dot().contains("\"db\" -> \"config\";"))
}
