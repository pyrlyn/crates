// The dependency graph of one container, as data. Built by the `@Container` macro (as
// `injectaGraph`), by `injecta-check` from source, or by hand in tests; checked by `issues()`.

/// How long a container keeps what an entry builds.
public enum Lifetime: String, Sendable, Hashable {
  /// Passed to the container's `init` (or a plain stored/computed member).
  case input
  /// Built once per container instance, then shared.
  case singleton
  /// Built on every access.
  case transient
  /// Read from the parent container (`parent.<name>`).
  case forward
}

/// One entry: what the container calls `name`, and the names it needs.
public struct Node: Sendable, Hashable {
  public var name: String
  public var lifetime: Lifetime
  /// The type as written in source, for messages and exports only.
  public var type: String
  /// Entries read while this one is built. These are the edges cycles and captive checks walk.
  public var dependencies: [String]
  /// `Lazy<T>` and `() -> T` needs. The entry must exist, but it is read later, so it is not an
  /// edge: it neither closes a cycle nor captures a transient.
  public var deferred: [String]

  public init(
    name: String, lifetime: Lifetime, type: String, dependencies: [String] = [],
    deferred: [String] = []
  ) {
    (self.name, self.lifetime, self.type, self.dependencies, self.deferred) =
      (name, lifetime, type, dependencies, deferred)
  }
}

/// A problem in a graph. `missing` and `cycle` are errors; `captive` is a warning.
public enum Issue: Sendable, Hashable, CustomStringConvertible {
  /// `node` needs `dependency`, which the container does not provide.
  case missing(node: String, dependency: String)
  /// The names in order; the first one repeats at the end.
  case cycle([String])
  /// A singleton keeps a transient alive for the container's life, so it is no longer transient.
  case captive(singleton: String, transient: String)

  public var isError: Bool {
    if case .captive = self { return false }
    return true
  }

  public var description: String {
    switch self {
    case .missing(let node, let dependency):
      "'\(node)' needs '\(dependency)', but the container has no entry named '\(dependency)'. "
        + "Add an init input `let \(dependency): <Type>`, a provider "
        + "`@Provides(.singleton) func make\(capitalized(dependency))(...) -> <Type>`, or, for an "
        + "@Injectable type, `@Singleton var \(dependency): <Type>`."
    case .cycle(let path):
      "dependency cycle: \(path.joined(separator: " -> ")). Break it in the design: move the "
        + "shared part into a third entry both depend on."
    case .captive(let singleton, let transient):
      "singleton '\(singleton)' captures transient '\(transient)': it is built once and kept. "
        + "Make '\(transient)' a singleton, or make '\(singleton)' transient, or depend on "
        + "`() -> \(transient)` (a new value each call) or `Lazy<\(transient)>` (one value, on "
        + "first read) instead of `\(transient)`."
    }
  }
}

func capitalized(_ name: String) -> String {
  name.prefix(1).uppercased() + name.dropFirst()
}

/// A container's entries and the checks every Injecta tool runs on them.
public struct Graph: Sendable, Hashable {
  public var container: String
  public var nodes: [Node]

  public init(container: String, nodes: [Node]) {
    (self.container, self.nodes) = (container, nodes)
  }

  public func node(named name: String) -> Node? { nodes.first { $0.name == name } }

  /// Every problem, errors first in source order, then warnings. Empty means the graph is sound.
  public func issues() -> [Issue] {
    let byName = Dictionary(nodes.map { ($0.name, $0) }, uniquingKeysWith: { first, _ in first })
    var found: [Issue] = []
    for node in nodes {
      for dependency in node.dependencies + node.deferred where byName[dependency] == nil {
        found.append(.missing(node: node.name, dependency: dependency))
      }
    }
    found += cycles(byName)
    for node in nodes where node.lifetime == .singleton {
      for dependency in node.dependencies where byName[dependency]?.lifetime == .transient {
        found.append(.captive(singleton: node.name, transient: dependency))
      }
    }
    return found
  }

  /// Each elementary cycle once, found by depth-first search with colours (white/grey/black).
  private func cycles(_ byName: [String: Node]) -> [Issue] {
    enum Colour { case grey, black }
    var colour: [String: Colour] = [:]
    var stack: [String] = []
    var found: [Issue] = []
    var seen: Set<[String]> = []

    func visit(_ name: String) {
      colour[name] = .grey
      stack.append(name)
      for dependency in byName[name]?.dependencies ?? [] where byName[dependency] != nil {
        switch colour[dependency] {
        case nil: visit(dependency)
        case .grey:
          let start = stack.lastIndex(of: dependency)!
          var path = Array(stack[start...])
          // Rotate so the same cycle found from another entry reads the same.
          let pivot = path.indices.min { path[$0] < path[$1] }!
          path = Array(path[pivot...] + path[..<pivot])
          if seen.insert(path).inserted { found.append(.cycle(path + [path[0]])) }
        case .black: break
        }
      }
      stack.removeLast()
      colour[name] = .black
    }

    for node in nodes where colour[node.name] == nil { visit(node.name) }
    return found
  }

  /// Graphviz DOT, one edge per dependency; singletons are boxes, transients ellipses.
  public func dot() -> String {
    var lines = ["digraph \"\(container)\" {"]
    for node in nodes {
      let shape =
        switch node.lifetime {
        case .singleton: "box"
        case .transient: "ellipse"
        case .input: "note"
        case .forward: "cds"
        }
      lines.append("  \"\(node.name)\" [shape=\(shape), label=\"\(node.name): \(escaped(node.type))\"];")
      for dependency in node.dependencies {
        lines.append("  \"\(node.name)\" -> \"\(dependency)\";")
      }
      for dependency in node.deferred {
        lines.append("  \"\(node.name)\" -> \"\(dependency)\" [style=dashed];")
      }
    }
    lines.append("}")
    return lines.joined(separator: "\n")
  }

  /// One line per entry, for logs and agents: `name: Type [lifetime] <- a, b`.
  public func describe() -> String {
    nodes.map { node in
      let names = node.dependencies + node.deferred.map { "~\($0)" }
      let needs = names.isEmpty ? "" : " <- " + names.joined(separator: ", ")
      return "\(node.name): \(node.type) [\(node.lifetime.rawValue)]\(needs)"
    }.joined(separator: "\n")
  }
}

private func escaped(_ text: String) -> String {
  text.reduce(into: "") { out, character in
    if character == "\"" { out += "\\\"" } else { out.append(character) }
  }
}
