// A dependency read on first use. Holding `Lazy<T>` (or `() -> T`) instead of `T` is how a
// singleton escapes a captive transient and how two entries break a cycle: the graph does not
// count either as an edge. Do not read `.value` from the initializer that received the `Lazy`;
// that re-enters the singleton cell while it is still being built.

import Synchronization

/// A value built on first read and then kept.
///
/// The cell is the same `Once` a singleton uses, so the first read is safe from several tasks
/// at once. The factory is not required to be `@Sendable`: a `@MainActor` container's closure
/// is isolated, and this wrapper is what such a container stores. Share a `Lazy` across
/// isolations only when `Value` is `Sendable` and the factory is safe to run from any of them.
public struct Lazy<Value>: @unchecked Sendable {
  private final class Cell: @unchecked Sendable {
    let once: Once<Value>
    let make: () -> Value

    init(_ make: @escaping () -> Value) {
      once = Once()
      self.make = make
    }
  }

  private let cell: Cell

  public init(_ make: @escaping () -> Value) {
    cell = Cell(make)
  }

  /// The value, built the first time this is read.
  public var value: Value { cell.once.get(cell.make) }
}
