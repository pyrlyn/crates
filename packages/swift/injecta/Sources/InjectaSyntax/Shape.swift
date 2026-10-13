// `Lazy<T>` and `() -> T`: the shapes a dependency takes when it is read later rather than while
// the dependent value is built.

import SwiftSyntax

/// What a parameter or stored-property type means as a container entry.
public struct DependencyShape {
  /// The entry's type. `Lazy<T>` and `() -> T` unwrap to `T`.
  public var type: String
  public var wrap: DependencyWrap
  public var sendableFactory: Bool

  /// `Lazy<T>` (or `Injecta.Lazy<T>`) and a synchronous non-throwing `() -> T` are deferred.
  /// `@Sendable` on the function is remembered so the generated closure can match. Anything else,
  /// including `() async -> T` and `() throws -> T`, is a direct dependency of the written type:
  /// the container has no async or throwing entries.
  public static func read(_ type: TypeSyntax, attributes: AttributeListSyntax = []) -> DependencyShape {
    var sendable = false
    let peeled = peel(type, sendable: &sendable)
    if attributes.attribute("Sendable") != nil { sendable = true }
    if let function = peeled.as(FunctionTypeSyntax.self),
      function.parameters.isEmpty,
      function.effectSpecifiers?.asyncSpecifier == nil,
      function.effectSpecifiers?.throwsClause == nil
    {
      return DependencyShape(
        type: function.returnClause.type.trimmedDescription, wrap: .factory, sendableFactory: sendable)
    }
    if let argument = lazyArgument(peeled) {
      return DependencyShape(type: argument, wrap: .lazy, sendableFactory: false)
    }
    return DependencyShape(type: type.trimmedDescription, wrap: .direct, sendableFactory: false)
  }
}

/// Drops type attributes (`@escaping`, `@Sendable`, `@MainActor`) so the shape underneath can be
/// classified. Records `@Sendable`.
private func peel(_ type: TypeSyntax, sendable: inout Bool) -> TypeSyntax {
  var current = type
  while let attributed = current.as(AttributedTypeSyntax.self) {
    for element in attributed.attributes {
      guard case .attribute(let attribute) = element else { continue }
      if attribute.simpleName == "Sendable" { sendable = true }
    }
    current = attributed.baseType
  }
  return current
}

/// The single generic argument of a type named `Lazy`, or `nil` when `type` is not one.
private func lazyArgument(_ type: TypeSyntax) -> String? {
  let arguments: GenericArgumentListSyntax?
  if let simple = type.as(IdentifierTypeSyntax.self), simple.name.text == "Lazy" {
    arguments = simple.genericArgumentClause?.arguments
  } else if let member = type.as(MemberTypeSyntax.self), member.name.text == "Lazy" {
    arguments = member.genericArgumentClause?.arguments
  } else {
    return nil
  }
  guard let arguments, arguments.count == 1, let argument = arguments.first else { return nil }
  return argument.argument.trimmedDescription
}
