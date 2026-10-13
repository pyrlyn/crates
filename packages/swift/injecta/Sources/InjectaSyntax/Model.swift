// What Injecta's attributes mean, read from one declaration's syntax. The macros read one
// declaration at a time; `injecta-check` reads every file of a target and joins the results, so
// both report the same rule with the same words.

import InjectaGraph
import SwiftSyntax

/// A rule broken at `node`. The macros turn it into a diagnostic, `injecta-check` into a
/// `file:line:column: error:` line.
public struct Problem: Error, CustomStringConvertible {
  public enum Severity: Sendable { case error, warning }
  public var message: String
  public var node: Syntax
  public var severity: Severity

  public init(_ message: String, at node: some SyntaxProtocol, severity: Severity = .error) {
    (self.message, self.node, self.severity) = (message, Syntax(node), severity)
  }

  public var description: String { message }
}

/// How a dependency is passed. `.lazy` and `.factory` are resolved later, so the graph does not
/// treat them as edges.
public enum DependencyWrap: Sendable {
  case direct, lazy, factory
}

/// A value a type or a provider needs: the argument label is the name the container must provide.
public struct Dependency {
  public var label: String
  /// The entry's type. For `Lazy<T>` and `() -> T` this is `T`, which is what the container stores.
  public var type: String
  public var wrap: DependencyWrap
  /// The factory was written `@Sendable () -> T`, so the generated closure is `@Sendable`.
  public var sendableFactory: Bool
  public var node: Syntax

  public init(
    label: String, type: String, node: Syntax, wrap: DependencyWrap = .direct,
    sendableFactory: Bool = false
  ) {
    (self.label, self.type, self.wrap, self.sendableFactory, self.node) =
      (label, type, wrap, sendableFactory, node)
  }

  /// Read later: not a cycle edge and not a captive capture.
  public var deferred: Bool { wrap != .direct }
}

/// The attribute names Injecta reacts to, without a module qualifier.
public enum Attribute {
  public static let injectable = "Injectable"
  public static let inject = "Inject"
  public static let container = "Container"
  public static let singleton = "Singleton"
  public static let transient = "Transient"
  public static let forward = "Forward"
  public static let provides = "Provides"
}

extension AttributeListSyntax {
  /// The attribute named `name` (`@Injecta.Singleton` counts as `Singleton`).
  public func attribute(_ name: String) -> AttributeSyntax? {
    for element in self {
      guard case .attribute(let attribute) = element else { continue }
      if attribute.simpleName == name { return attribute }
    }
    return nil
  }

  /// The first global actor among the attributes (`@MainActor`), recognised by the `Actor`
  /// suffix: a macro cannot resolve names, and every global actor in practice follows it.
  public var globalActor: String? {
    for element in self {
      guard case .attribute(let attribute) = element else { continue }
      let name = attribute.attributeName.trimmedDescription
      if name.hasSuffix("Actor") { return name }
    }
    return nil
  }
}

extension AttributeSyntax {
  public var simpleName: String {
    let full = attributeName.trimmedDescription
    return full.split(separator: ".").last.map(String.init) ?? full
  }
}

extension DeclModifierListSyntax {
  /// The access level to repeat on generated members, with its trailing space (`"public "`).
  /// `open` becomes `public`; `private`/`fileprivate` become internal, since generated members
  /// are read from other declarations.
  public var accessPrefix: String {
    for modifier in self {
      switch modifier.name.tokenKind {
      case .keyword(.public), .keyword(.open): return "public "
      case .keyword(.package): return "package "
      default: continue
      }
    }
    return ""
  }

  public func contains(_ keyword: Keyword) -> Bool {
    contains { $0.name.tokenKind == .keyword(keyword) }
  }
}

public func lowerFirst(_ text: some StringProtocol) -> String {
  text.prefix(1).lowercased() + text.dropFirst()
}
