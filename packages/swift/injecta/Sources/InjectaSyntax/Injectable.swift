// `@Injectable`: which initializer the container calls, and what it needs.

import SwiftSyntax

/// A type the container can build by itself.
public struct InjectableModel {
  public enum Kind: Sendable { case `struct`, `class`, actor }
  public var typeName: String
  public var access: String
  public var kind: Kind
  public var globalActor: String?
  /// In parameter order; the labels are the names the container must provide.
  public var dependencies: [Dependency]
}

public enum InjectableReader {
  /// Reads `decl` (the type `@Injectable` is attached to). Returns `nil` with the problems when
  /// the type cannot be injected.
  public static func read(_ decl: some DeclGroupSyntax) -> (InjectableModel?, [Problem]) {
    let kind: InjectableModel.Kind
    let name: TokenSyntax
    let generics: GenericParameterClauseSyntax?
    if let decl = decl.as(StructDeclSyntax.self) {
      (kind, name, generics) = (.struct, decl.name, decl.genericParameterClause)
    } else if let decl = decl.as(ClassDeclSyntax.self) {
      (kind, name, generics) = (.class, decl.name, decl.genericParameterClause)
    } else if let decl = decl.as(ActorDeclSyntax.self) {
      (kind, name, generics) = (.actor, decl.name, decl.genericParameterClause)
    } else {
      return (nil, [Problem("@Injectable applies to a struct, class or actor", at: decl)])
    }
    if let generics {
      return (
        nil,
        [
          Problem(
            "@Injectable does not support generic types: Swift cannot nest the generated "
              + "`Needs` protocol in them. Register a concrete type with a "
              + "`@Provides(.singleton) func make<Name>(...)` provider instead",
            at: generics)
        ]
      )
    }
    let (dependencies, problems) = initializerDependencies(decl, kind: kind)
    guard let dependencies else { return (nil, problems) }
    let model = InjectableModel(
      typeName: name.trimmedDescription,
      access: decl.modifiers.accessPrefix,
      kind: kind,
      globalActor: decl.attributes.globalActor,
      dependencies: dependencies)
    return (model, problems)
  }

  /// The initializer marked `@Inject`, else the only one, else a struct's memberwise one.
  private static func initializerDependencies(
    _ decl: some DeclGroupSyntax, kind: InjectableModel.Kind
  ) -> ([Dependency]?, [Problem]) {
    let inits = decl.memberBlock.members.compactMap { $0.decl.as(InitializerDeclSyntax.self) }
    let marked = inits.filter { $0.attributes.attribute(Attribute.inject) != nil }
    let chosen: InitializerDeclSyntax
    if marked.count > 1 {
      return (nil, marked.dropFirst().map { Problem("only one initializer may be @Inject", at: $0) })
    } else if let only = marked.first ?? (inits.count == 1 ? inits.first : nil) {
      chosen = only
    } else if inits.isEmpty {
      return (memberwise(decl, kind: kind), [])
    } else {
      return (
        nil,
        [
          Problem(
            "this type has \(inits.count) initializers; mark the one the container calls "
              + "with @Inject", at: decl.memberBlock)
        ]
      )
    }
    var problems: [Problem] = []
    if chosen.optionalMark != nil {
      problems.append(Problem("an injected initializer cannot be failable", at: chosen))
    }
    if let effects = chosen.signature.effectSpecifiers,
      effects.asyncSpecifier != nil || effects.throwsClause != nil
    {
      problems.append(
        Problem(
          "an injected initializer cannot be async or throw: build the value first and pass it "
            + "to the container's init as an input", at: effects))
    }
    var dependencies: [Dependency] = []
    for parameter in chosen.signature.parameterClause.parameters
    where parameter.defaultValue == nil {
      if parameter.firstName.tokenKind == .wildcard {
        problems.append(
          Problem(
            "an injected parameter needs an argument label: the label is the container entry "
              + "it is read from", at: parameter))
        continue
      }
      if parameter.ellipsis != nil {
        problems.append(Problem("a variadic parameter cannot be injected", at: parameter))
        continue
      }
      let shape = DependencyShape.read(parameter.type, attributes: parameter.attributes)
      dependencies.append(
        Dependency(
          label: parameter.firstName.trimmedDescription, type: shape.type, node: Syntax(parameter),
          wrap: shape.wrap, sendableFactory: shape.sendableFactory))
    }
    return (problems.contains { $0.severity == .error } ? nil : dependencies, problems)
  }

  /// A struct without initializers gets the memberwise one: its stored properties without a
  /// default value, in order. A class or actor without one needs nothing.
  private static func memberwise(
    _ decl: some DeclGroupSyntax, kind: InjectableModel.Kind
  ) -> [Dependency] {
    guard kind == .struct else { return [] }
    var dependencies: [Dependency] = []
    for member in decl.memberBlock.members {
      guard let variable = member.decl.as(VariableDeclSyntax.self),
        !variable.modifiers.contains(.static)
      else { continue }
      for binding in variable.bindings where binding.accessorBlock == nil {
        guard let type = binding.typeAnnotation?.type,
          let name = binding.pattern.as(IdentifierPatternSyntax.self)?.identifier
        else { continue }
        // A defaulted `var` keeps its default in the memberwise init; a defaulted `let` is not a
        // parameter at all. Either way the container passes nothing.
        if binding.initializer != nil { continue }
        let shape = DependencyShape.read(type)
        dependencies.append(
          Dependency(
            label: name.trimmedDescription, type: shape.type, node: Syntax(binding), wrap: shape.wrap,
            sendableFactory: shape.sendableFactory))
      }
    }
    return dependencies
  }
}
