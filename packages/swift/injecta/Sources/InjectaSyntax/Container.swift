// `@Container`: the entries of one container, read from its members.

import InjectaGraph
import SwiftSyntax

/// One name a container provides.
public struct Entry {
  public var name: String
  public var lifetime: Lifetime
  /// The type as written (`any CoreClient`, `SettingsStore`).
  public var type: String
  /// What a provider function or `@Forward` needs; `nil` when the entry is auto-wired, so its
  /// needs are the type's `@Injectable` initializer, unknown to a single declaration.
  public var dependencies: [Dependency]?
  /// The provider function (`makeCore`), for entries declared with one.
  public var make: String?
  /// `@Singleton(.eager)`: built in the container's init instead of on first use.
  public var eager: Bool
  /// An input with no value: a parameter of the generated `init`.
  public var isInitParameter: Bool
  public var node: Syntax

  public var isAutoWired: Bool { dependencies == nil && (lifetime == .singleton || lifetime == .transient) }
}

public struct ContainerModel {
  public var name: String
  public var access: String
  public var globalActor: String?
  public var entries: [Entry]

  public var parameters: [Entry] { entries.filter(\.isInitParameter) }

  /// The graph as far as this declaration knows it. `autoWired` supplies the needs of an
  /// `@Injectable` type when the caller has read it (`injecta-check`); the macro passes `nil`
  /// and leaves those edges to the generated `injectaGraph` and to `injecta-check`.
  /// `Lazy<T>` and `() -> T` needs come back as deferred, not as edges.
  public func graph(
    autoWired: (String) -> (edges: [String], deferred: [String])? = { _ in nil }
  ) -> Graph {
    Graph(
      container: name,
      nodes: entries.map { entry in
        let edges: [String]
        let deferred: [String]
        if let dependencies = entry.dependencies {
          edges = dependencies.filter { !$0.deferred }.map(\.label)
          deferred = dependencies.filter(\.deferred).map(\.label)
        } else if let wired = autoWired(entry.type) {
          (edges, deferred) = (wired.edges, wired.deferred)
        } else {
          (edges, deferred) = ([], [])
        }
        return Node(
          name: entry.name, lifetime: entry.lifetime, type: entry.type, dependencies: edges,
          deferred: deferred)
      })
  }

  /// The checks a single declaration can make: missing names of provider functions, cycles
  /// among them, captive transients. `autoWired` as in `graph(autoWired:)`.
  public func problems(
    autoWired: (String) -> (edges: [String], deferred: [String])? = { _ in nil }
  ) -> [Problem] {
    let graph = graph(autoWired: autoWired)
    var problems: [Problem] = []
    for issue in graph.issues() {
      switch issue {
      case .missing(let name, let dependency):
        guard let entry = entries.first(where: { $0.name == name }) else { continue }
        let node = entry.dependencies?.first { $0.label == dependency }?.node ?? entry.node
        problems.append(Problem(issue.description, at: node))
      case .cycle(let path):
        guard let entry = entries.first(where: { $0.name == path[0] }) else { continue }
        problems.append(Problem(issue.description, at: entry.node))
      case .captive(let singleton, _):
        guard let entry = entries.first(where: { $0.name == singleton }) else { continue }
        problems.append(Problem(issue.description, at: entry.node, severity: .warning))
      }
    }
    return problems
  }
}

public enum ContainerReader {
  public static func read(_ decl: some DeclGroupSyntax) -> (ContainerModel?, [Problem]) {
    // A final class: its singleton cells are noncopyable and live inside the object, and every
    // holder of the container must share them.
    guard let classDecl = decl.as(ClassDeclSyntax.self) else {
      return (nil, [Problem("@Container applies to a `final class` (singletons are shared by reference)", at: decl)])
    }
    if let generics = classDecl.genericParameterClause {
      return (nil, [Problem("a @Container cannot be generic", at: generics)])
    }
    let name = classDecl.name.trimmedDescription
    var problems: [Problem] = []
    if !classDecl.modifiers.contains(.final) {
      problems.append(Problem("a @Container must be `final`: a subclass cannot add entries", at: classDecl.classKeyword))
    }
    var entries: [Entry] = []
    for member in decl.memberBlock.members {
      if let variable = member.decl.as(VariableDeclSyntax.self) {
        read(variable, into: &entries, &problems)
      } else if let function = member.decl.as(FunctionDeclSyntax.self) {
        read(function, into: &entries, &problems)
      }
    }
    var seen: Set<String> = []
    for entry in entries where !seen.insert(entry.name).inserted {
      problems.append(
        Problem(
          "'\(entry.name)' is declared twice; each entry needs its own name", at: entry.node))
    }
    let hasParent = entries.contains { $0.name == "parent" && $0.lifetime == .input }
    for index in entries.indices where entries[index].lifetime == .forward && !hasParent {
      problems.append(
        Problem(
          "@Forward reads `parent.\(entries[index].name)`; declare the parent container as an "
            + "input `let parent: <ParentContainer>`", at: entries[index].node))
      // Reported once, here, rather than again as a generic missing entry.
      entries[index].dependencies = []
    }
    let model = ContainerModel(
      name: name, access: decl.modifiers.accessPrefix, globalActor: decl.attributes.globalActor,
      entries: entries)
    return (model, problems + model.problems())
  }

  private static func read(
    _ variable: VariableDeclSyntax, into entries: inout [Entry], _ problems: inout [Problem]
  ) {
    if variable.modifiers.contains(.static) { return }
    let attributes = variable.attributes
    let lifetime: Lifetime? =
      if attributes.attribute(Attribute.singleton) != nil { .singleton }
      else if attributes.attribute(Attribute.transient) != nil { .transient }
      else if attributes.attribute(Attribute.forward) != nil { .forward }
      else { nil }
    guard let lifetime else {
      // A plain member is something the container already provides: an init input when it is
      // stored without a value, a constant or computed property otherwise.
      for binding in variable.bindings {
        guard let name = binding.pattern.as(IdentifierPatternSyntax.self)?.identifier else { continue }
        let isParameter = binding.initializer == nil && binding.accessorBlock == nil
        guard let type = binding.typeAnnotation?.type else {
          if isParameter {
            problems.append(Problem("an input needs a type annotation", at: binding))
          }
          continue
        }
        entries.append(
          Entry(
            name: name.trimmedDescription, lifetime: .input, type: type.trimmedDescription,
            dependencies: [], make: nil, eager: false, isInitParameter: isParameter,
            node: Syntax(binding)))
      }
      return
    }
    guard variable.bindings.count == 1, let binding = variable.bindings.first,
      let name = binding.pattern.as(IdentifierPatternSyntax.self)?.identifier
    else {
      problems.append(Problem("declare one entry per `var`", at: variable))
      return
    }
    if variable.bindingSpecifier.tokenKind != .keyword(.var) {
      problems.append(Problem("an entry is a `var` (the macro gives it a getter)", at: variable.bindingSpecifier))
    }
    guard let type = binding.typeAnnotation?.type else {
      problems.append(Problem("an entry needs a type annotation", at: binding))
      return
    }
    if binding.initializer != nil || binding.accessorBlock != nil {
      problems.append(
        Problem(
          "an entry has no value or body: the container builds it. For custom construction use "
            + "a provider, `@Provides(.\(lifetime == .singleton ? "singleton" : "transient")) func "
            + "make\(name.text.prefix(1).uppercased() + name.text.dropFirst())(...) -> "
            + "\(type.trimmedDescription)`", at: binding))
    }
    let typeText = type.trimmedDescription
    if lifetime != .forward && !isConcreteNominal(type) {
      problems.append(
        Problem(
          "'\(typeText)' cannot be auto-wired: only a concrete @Injectable type can. Use a "
            + "provider: `@Provides(.\(lifetime == .singleton ? "singleton" : "transient")) func "
            + "make\(name.text.prefix(1).uppercased() + name.text.dropFirst())(...) -> \(typeText)`",
          at: type))
    }
    entries.append(
      Entry(
        name: name.trimmedDescription, lifetime: lifetime, type: typeText,
        dependencies: lifetime == .forward ? [Dependency(label: "parent", type: "", node: Syntax(binding))] : nil,
        make: nil, eager: isEager(attributes), isInitParameter: false, node: Syntax(binding)))
  }

  private static func read(
    _ function: FunctionDeclSyntax, into entries: inout [Entry], _ problems: inout [Problem]
  ) {
    let attributes = function.attributes
    if let misplaced = attributes.attribute(Attribute.singleton) ?? attributes.attribute(Attribute.transient) {
      problems.append(
        Problem(
          "@\(misplaced.simpleName) is for `var` entries; a function is a provider: "
            + "`@Provides(.\(misplaced.simpleName == "Singleton" ? "singleton" : "transient"))`",
          at: misplaced))
      return
    }
    guard let provides = attributes.attribute(Attribute.provides) else { return }
    let provision = Provision(provides)
    let lifetime = provision.lifetime
    let functionName = function.name.text
    guard functionName.hasPrefix("make"), functionName.count > 4,
      functionName.dropFirst(4).first?.isUppercase == true
    else {
      problems.append(
        Problem(
          "a provider is named `make<Entry>` (`makeCore` provides `core`)", at: function.name))
      return
    }
    guard let returnType = function.signature.returnClause?.type else {
      problems.append(Problem("a provider returns the entry's type", at: function.signature))
      return
    }
    if let effects = function.signature.effectSpecifiers,
      effects.asyncSpecifier != nil || effects.throwsClause != nil
    {
      problems.append(
        Problem(
          "a provider cannot be async or throw: build the value before the container and pass "
            + "it to init as an input", at: effects))
    }
    var dependencies: [Dependency] = []
    for parameter in function.signature.parameterClause.parameters {
      if parameter.firstName.tokenKind == .wildcard {
        problems.append(
          Problem(
            "a provider parameter needs a label: the label is the entry it is read from",
            at: parameter))
        continue
      }
      let shape = DependencyShape.read(parameter.type, attributes: parameter.attributes)
      dependencies.append(
        Dependency(
          label: parameter.firstName.trimmedDescription, type: shape.type, node: Syntax(parameter),
          wrap: shape.wrap, sendableFactory: shape.sendableFactory))
    }
    entries.append(
      Entry(
        name: lowerFirst(functionName.dropFirst(4)), lifetime: lifetime,
        type: returnType.trimmedDescription, dependencies: dependencies, make: functionName,
        eager: provision == .eagerSingleton, isInitParameter: false, node: Syntax(function)))
  }

  private static func isEager(_ attributes: AttributeListSyntax) -> Bool {
    guard let attribute = attributes.attribute(Attribute.singleton),
      case .argumentList(let arguments) = attribute.arguments
    else { return false }
    return arguments.contains { $0.expression.trimmedDescription.hasSuffix("eager") }
  }

  /// `Foo` or `Module.Foo` (no `any`, `some`, optional, tuple, function or generic arguments):
  /// what can have a nested `Needs` protocol.
  private static func isConcreteNominal(_ type: TypeSyntax) -> Bool {
    if let simple = type.as(IdentifierTypeSyntax.self) { return simple.genericArgumentClause == nil }
    if let member = type.as(MemberTypeSyntax.self) { return member.genericArgumentClause == nil }
    return false
  }
}

/// The argument of `@Provides(...)`, read from its text (a macro sees no types).
public enum Provision: Equatable {
  case singleton, eagerSingleton, transient

  public init(_ attribute: AttributeSyntax) {
    guard case .argumentList(let arguments) = attribute.arguments, let first = arguments.first else {
      self = .singleton
      return
    }
    let text = first.expression.trimmedDescription
    if text.hasSuffix("transient") { self = .transient }
    else if text.hasSuffix("eagerSingleton") { self = .eagerSingleton }
    else { self = .singleton }
  }

  var lifetime: Lifetime { self == .transient ? .transient : .singleton }
}
