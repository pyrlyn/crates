// Every Injecta declaration in a set of files, for `injecta-check`: the whole-target view a
// macro cannot have.

import InjectaGraph
import SwiftSyntax

/// What one target declares.
public struct TargetScan {
  public struct Located<Value> {
    public var value: Value
    public var file: String
    public var converter: SourceLocationConverter
  }

  public var injectables: [String: Located<InjectableModel>] = [:]
  public var containers: [Located<ContainerModel>] = []

  public init() {}

  /// Adds the declarations of one parsed file.
  public mutating func add(_ tree: SourceFileSyntax, file: String) {
    let converter = SourceLocationConverter(fileName: file, tree: tree)
    let visitor = DeclarationVisitor(viewMode: .sourceAccurate)
    visitor.walk(tree)
    for decl in visitor.injectables {
      if let model = InjectableReader.read(decl).0 {
        injectables[model.typeName] = Located(value: model, file: file, converter: converter)
      }
    }
    for decl in visitor.containers {
      if let model = ContainerReader.read(decl).0 {
        containers.append(Located(value: model, file: file, converter: converter))
      }
    }
  }

  /// The needs of an auto-wired type, when this target declares it (`Module.Type` matches `Type`).
  /// Deferred needs (`Lazy<T>`, `() -> T`) are listed apart from the edges.
  public func needs(of type: String) -> (edges: [String], deferred: [String])? {
    let simple = type.split(separator: ".").last.map(String.init) ?? type
    guard let dependencies = injectables[simple]?.value.dependencies else { return nil }
    return (
      edges: dependencies.filter { !$0.deferred }.map(\.label),
      deferred: dependencies.filter(\.deferred).map(\.label))
  }

  /// One diagnostic line per problem only the whole target reveals (the macro already reports
  /// the rest): `path:line:column: error: injecta: message`.
  public func diagnostics() -> [(line: String, isError: Bool)] {
    var lines: [(String, Bool)] = []
    for container in containers {
      let local = Set(container.value.problems().map(key))
      for problem in container.value.problems(autoWired: needs(of:)) where !local.contains(key(problem)) {
        let location = problem.node.startLocation(converter: container.converter)
        let severity = problem.severity == .error ? "error" : "warning"
        lines.append((
          "\(container.file):\(location.line):\(location.column): \(severity): injecta: "
            + "\(container.value.name): \(problem.message)",
          problem.severity == .error
        ))
      }
    }
    return lines
  }

  /// The `Needs` conformance of every auto-wired type, one line each: what `@Container` cannot
  /// add itself. A `@MainActor` container gets an isolated conformance (SE-0470).
  public func conformances() -> [String] {
    var lines: [String] = []
    for container in containers {
      let isolation = container.value.globalActor.map { "@\($0) " } ?? ""
      var seen: Set<String> = []
      for entry in container.value.entries where entry.isAutoWired && seen.insert(entry.type).inserted {
        lines.append("extension \(container.value.name): \(isolation)\(entry.type).Needs {}")
      }
    }
    return lines
  }

  private func key(_ problem: Problem) -> String {
    "\(problem.node.position.utf8Offset):\(problem.message)"
  }
}

private final class DeclarationVisitor: SyntaxVisitor {
  var injectables: [any DeclGroupSyntax] = []
  var containers: [any DeclGroupSyntax] = []

  private func sort(_ decl: some DeclGroupSyntax) {
    if decl.attributes.attribute(Attribute.injectable) != nil { injectables.append(decl) }
    if decl.attributes.attribute(Attribute.container) != nil { containers.append(decl) }
  }

  override func visit(_ node: StructDeclSyntax) -> SyntaxVisitorContinueKind { sort(node); return .visitChildren }
  override func visit(_ node: ClassDeclSyntax) -> SyntaxVisitorContinueKind { sort(node); return .visitChildren }
  override func visit(_ node: ActorDeclSyntax) -> SyntaxVisitorContinueKind { sort(node); return .visitChildren }
}
