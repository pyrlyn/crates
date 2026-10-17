import Foundation
import InjectaGraph
import SwiftSyntax
import SwiftSyntaxBuilder
import SwiftSyntaxMacros

/// `@Container`: the init over the inputs, overrides, eager singletons, the graph, and one
/// `Needs` conformance per auto-wired type.
public struct ContainerMacro: MemberMacro, ExtensionMacro {
  public static func expansion(
    of node: AttributeSyntax,
    providingMembersOf declaration: some DeclGroupSyntax,
    conformingTo protocols: [TypeSyntax],
    in context: some MacroExpansionContext
  ) throws -> [DeclSyntax] {
    let (model, problems) = ContainerReader.read(declaration)
    // Reported here only: the extension role reads the same declaration and stays silent.
    context.report(problems)
    guard let model else { return [] }
    let access = model.access
    let isolated = model.globalActor != nil
    let nonisolated = isolated ? "nonisolated " : ""

    var overrideFields: [String] = []
    var seeds: [String] = []
    for entry in model.entries {
      switch entry.lifetime {
      case .singleton:
        overrideFields.append("  \(access)var \(entry.name): (\(storable(entry.type)))? = nil")
        seeds.append("    if let value = overrides.\(entry.name) { _injecta_\(entry.name).seed(value) }")
      case .transient:
        let sendable = isolated ? "" : "@Sendable "
        overrideFields.append("  \(access)var \(entry.name): (\(sendable)() -> \(storable(entry.type)))? = nil")
      case .input, .forward:
        continue
      }
    }
    let overridesSendable = isolated ? "" : ": Sendable"
    let overrides: DeclSyntax = """
      /// Replacements for tests and previews. A singleton set here is the value from the start
      /// (no cost on the resolve path); a transient set here is called instead of its provider.
      \(raw: access)struct Overrides\(raw: overridesSendable) {
      \(raw: overrideFields.joined(separator: "\n"))
        \(raw: access)init() {}
      }
      """
    let storage: DeclSyntax = "private let _injectaOverrides: Overrides"

    let parameters = model.parameters.map { "\($0.name): \(storable($0.type))" } + ["overrides: Overrides = Overrides()"]
    let assignments = model.parameters.map { "    self.\($0.name) = \($0.name)" }
    let eager = model.entries.filter { $0.lifetime == .singleton && $0.eager }.map { "    _ = self.\($0.name)" }
    // Compared with `injectaGraph`'s auto-wired entries: each must conform to `Type.Needs`, which
    // is the line the plugin generates. Debug only, and after every stored property is set.
    var seen: Set<String> = []
    let checks = model.entries.filter(\.isAutoWired).compactMap { entry -> String? in
      guard seen.insert(entry.type).inserted else { return nil }
      return """
          #if DEBUG
          if !(self is \(entry.type).Needs) { Injecta.missingPlugin("\(entry.type)") }
          #endif
      """
    }
    let body = (assignments + ["    self._injectaOverrides = overrides"] + seeds + checks + eager)
      .joined(separator: "\n")
    let initializer: DeclSyntax = """
      \(raw: access)init(\(raw: parameters.joined(separator: ", "))) {
      \(raw: body)
      }
      """

    let nodes = model.entries.map { entry -> String in
      let needs: String
      let deferred: String
      if let dependencies = entry.dependencies {
        needs = list(dependencies.filter { !$0.deferred }.map(\.label))
        deferred = list(dependencies.filter(\.deferred).map(\.label))
      } else {
        needs = "\(entry.type).injectaDependencies"
        deferred = "\(entry.type).injectaDeferred"
      }
      return "      Injecta.Node(name: \"\(entry.name)\", lifetime: .\(entry.lifetime.rawValue), "
        + "type: \(literal(entry.type)), dependencies: \(needs), deferred: \(deferred)),"
    }
    let graph: DeclSyntax = """
      /// Every entry and what it needs; check it in a test with `injectaGraph.issues()`.
      \(raw: access)\(raw: nonisolated)static let injectaGraph = Injecta.Graph(
        container: "\(raw: model.name)",
        nodes: [
      \(raw: nodes.joined(separator: "\n"))
        ])
      """
    return [overrides, storage, initializer, graph]
  }

  public static func expansion(
    of node: AttributeSyntax,
    attachedTo declaration: some DeclGroupSyntax,
    providingExtensionsOf type: some TypeSyntaxProtocol,
    conformingTo protocols: [TypeSyntax],
    in context: some MacroExpansionContext
  ) throws -> [ExtensionDeclSyntax] {
    // The `Needs` conformances of auto-wired types are not added here: a macro may only add
    // conformances its declaration lists, so `injecta-check` generates them for the target.
    guard !protocols.isEmpty, ContainerReader.read(declaration).0 != nil else { return [] }
    return [try ExtensionDeclSyntax("extension \(raw: type.trimmedDescription): Injecta.Container {}")]
  }

  private static func literal(_ text: String) -> String {
    "\"" + text.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\""
  }

  private static func list(_ names: [String]) -> String {
    "[" + names.map { "\"\($0)\"" }.joined(separator: ", ") + "]"
  }
}
