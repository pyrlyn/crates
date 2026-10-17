// Injecta's public surface: the macros, the two protocols their output conforms to, and the graph
// model re-exported from `InjectaGraph`.

@_exported import InjectaGraph

/// What a `@Provides` function's value is.
public enum Provision: Sendable {
  /// One value per container instance, built on first access.
  case singleton
  /// One value per container instance, built in the container's `init`.
  case eagerSingleton
  /// A new value on every access.
  case transient
}

/// When a singleton is built.
public enum Build: Sendable {
  /// On first access (the default).
  case lazy
  /// In the container's `init`, so the cost is paid at launch and the first access is already
  /// the cached path. Use it for values whose construction must happen exactly at startup.
  case eager
}

/// A type the container builds by itself. Conformance comes from `@Injectable`; do not write it
/// by hand.
public protocol Injectable {
  /// The argument labels of the injected initializer, in order: the entries it reads.
  static var injectaDependencies: [String] { get }
}

/// What a debug build reports when an auto-wired entry has no `Needs` conformance.
///
/// The conformance is the line `InjectaCheckPlugin` writes. Without the plugin, and without that
/// line written by hand, a debug build compiles and traps here on `init`, naming the setup step.
/// A release build still fails at compile time, on `Type(injecting: self)`.
public func missingPluginMessage(_ type: String) -> String {
  "\(type) could not be built: this container does not conform to \(type).Needs. "
    + "Attach InjectaCheckPlugin to the target that declares the container "
    + "(GUIDE.md, \"The plugin\"), or add `extension <Container>: \(type).Needs {}` by hand."
}

/// Traps with `missingPluginMessage(_:)`. Generated into a container's `init` and getters, in
/// debug builds only.
public func missingPlugin(_ type: String) -> Never {
  fatalError(missingPluginMessage(type))
}

/// A type declared with `@Container`. Conformance comes from the macro.
public protocol Container {
  /// Every entry, its lifetime and what it needs: for `graph.issues()` in a test, for
  /// `graph.dot()` and `graph.describe()`.
  static var injectaGraph: Graph { get }
}

// MARK: - Macros

/// Lets a container build this type through its initializer.
///
/// The container calls the initializer marked `@Inject`, else the only initializer, else a
/// struct's memberwise initializer. Each argument label without a default value is an entry the
/// container must provide under that name. A `Lazy<T>` or `() -> T` parameter reads entry `T`
/// later (not an edge). Generates a nested `Needs` protocol (one `var` per label),
/// `init(injecting:)`, `injectaDependencies` and `injectaDeferred`.
@attached(extension, conformances: Injectable, names: named(Needs), named(init(injecting:)), named(injectaDependencies), named(injectaDeferred))
public macro Injectable() = #externalMacro(module: "InjectaMacros", type: "InjectableMacro")

/// Marks the initializer `@Injectable` calls when a type has more than one.
@attached(peer)
public macro Inject() = #externalMacro(module: "InjectaMacros", type: "InjectMacro")

/// Declares a container: a `final class` whose members are its entries.
///
/// - `let name: Type` without a value: an input, a parameter of the generated `init`.
/// - `@Singleton var name: Type` / `@Transient var name: Type`: built from `Type`'s
///   `@Injectable` initializer.
/// - `@Provides(.singleton) func makeName(a: A, b: B) -> Type { ... }`: a provider; its
///   labels are entries (`.transient` and `.eagerSingleton` also work).
/// - `@Forward var name: Type`: read from the input `parent` (a parent container).
///
/// Generates `init(<inputs>, overrides: Overrides = Overrides())`, the `Overrides` struct and
/// `injectaGraph`, and checks the graph it can see: a provider label with no entry, a cycle and a
/// captive transient are compile errors (warning for captive) at the offending line.
///
/// An auto-wired type needs one more line, `extension <Container>: <Type>.Needs {}`, because a
/// macro may only add conformances it declares up front. `InjectaCheckPlugin` generates these
/// lines for the whole target (and checks the graph across files); without the plugin, write them.
@attached(member, names: named(init), named(Overrides), named(injectaGraph), named(_injectaOverrides))
@attached(extension, conformances: Container)
public macro Container() = #externalMacro(module: "InjectaMacros", type: "ContainerMacro")

/// One value per container instance, built on first access (or in `init` with `.eager`), from
/// the type's `@Injectable` initializer.
@attached(accessor)
@attached(peer, names: prefixed(_injecta_))
public macro Singleton(_ build: Build = .lazy) = #externalMacro(module: "InjectaMacros", type: "SingletonMacro")

/// A new value on every access, from the type's `@Injectable` initializer.
@attached(accessor)
public macro Transient() = #externalMacro(module: "InjectaMacros", type: "TransientMacro")

/// A provider function `make<Name>(label: Type, ...) -> Type`: generates the entry `name`, which
/// calls the function with the entries its labels name. For protocol types, values from other
/// modules, and anything that needs custom construction.
@attached(peer, names: arbitrary)
public macro Provides(_ provision: Provision = .singleton) = #externalMacro(module: "InjectaMacros", type: "ProvidesMacro")

/// The parent container's entry of the same name (`parent.name`).
@attached(accessor)
public macro Forward() = #externalMacro(module: "InjectaMacros", type: "ForwardMacro")
