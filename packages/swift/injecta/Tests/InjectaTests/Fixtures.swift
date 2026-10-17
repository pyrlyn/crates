// A small app graph used by the runtime tests: protocols with live and fake implementations,
// auto-wired types, providers, a child scope and a main-actor container.

import Injecta
import Synchronization

/// Counts constructions, so tests can tell a singleton from a transient.
final class Counter: Sendable {
  private let value = Mutex(0)
  func bump() -> Int { value.withLock { $0 += 1; return $0 } }
  var count: Int { value.withLock { $0 } }
}

protocol Logger: Sendable { var level: Int { get } }
struct ConsoleLogger: Logger { let level: Int }
struct SilentLogger: Logger { let level = 0 }

struct Config: Sendable, Equatable {
  var poolSize: Int
}

@Injectable
final class Database: Sendable {
  let config: Config
  let logger: any Logger
  init(config: Config, logger: any Logger, counter: Counter) {
    (self.config, self.logger) = (config, logger)
    _ = counter.bump()
  }
}

@Injectable
struct UserRepo: Sendable {
  let db: Database
  let logger: any Logger
}

@Injectable
struct UserService: Sendable {
  let repo: UserRepo
  let logger: any Logger
  var retries = 3
}

@Injectable
final class Handler: Sendable {
  let service: UserService
  let tag: String

  @Inject
  init(service: UserService, tag: String = "default") { (self.service, self.tag) = (service, tag) }

  init(fake: UserService) { (self.service, self.tag) = (fake, "fake") }
}

@Container
final class AppGraph: Sendable {
  let config: Config
  let counter: Counter

  @Provides(.singleton) func makeLogger(config: Config) -> any Logger { ConsoleLogger(level: config.poolSize) }
  @Singleton var db: Database
  @Transient var repo: UserRepo
  @Transient var service: UserService
  @Transient var handler: Handler
}

/// A per-request scope: its own singletons, everything else from the app.
@Container
final class RequestGraph: Sendable {
  let parent: AppGraph
  let requestID: Int

  @Forward var db: Database
  @Forward var logger: any Logger
  @Singleton var repo: UserRepo
  @Provides(.transient) func makeTrace(requestID: Int) -> String { "req-\(requestID)" }
}

/// Built at init, so its cost is paid at launch.
@Container
final class EagerGraph: Sendable {
  let config: Config
  let counter: Counter
  @Provides(.singleton) func makeLogger(config: Config) -> any Logger { SilentLogger() }
  @Singleton(.eager) var db: Database
}

// MARK: - Main-actor graph, as in a SwiftUI app

@MainActor
@Injectable
final class SettingsModel {
  let logger: any Logger
  var title = "Settings"
  init(logger: any Logger) { self.logger = logger }
}

@MainActor
@Container
final class UIGraph {
  let logger: any Logger
  @Singleton var settings: SettingsModel
}

// MARK: - Deferred dependencies: a factory escapes a captive transient, Lazy breaks a cycle

@Injectable
final class Clock: Sendable {
  let id: Int
  init(counter: Counter) { id = counter.bump() }
}

@Injectable
final class Cache: Sendable {
  let clock: @Sendable () -> Clock
  init(clock: @escaping @Sendable () -> Clock) { self.clock = clock }
}

@Injectable
final class Left: Sendable {
  let right: Lazy<Right>
  init(right: Lazy<Right>) { self.right = right }
}

@Injectable
final class Right: Sendable {
  let left: Left
  init(left: Left) { self.left = left }
}

@Container
final class FactoryGraph: Sendable {
  let counter: Counter
  @Transient var clock: Clock
  @Singleton var cache: Cache
  @Singleton var left: Left
  @Singleton var right: Right
}

/// The same factory, written as a provider instead of an `@Injectable` initializer.
@Container
final class ProviderFactoryGraph: Sendable {
  let counter: Counter
  @Transient var clock: Clock
  @Provides(.singleton) func makeCache(clock: @escaping @Sendable () -> Clock) -> Cache { Cache(clock: clock) }
}
