// The same graph wired by hand and by each library, in the style each library documents.

import Dependencies
import FactoryKit
import Injecta
import Swinject

// MARK: - Hand-written baseline: singletons built in init, transients are init calls.

final class Manual: Sendable {
  let config: Config
  let logger: any Logger
  let db: Database
  let cache: Cache

  init(config: Config) {
    self.config = config
    logger = ConsoleLogger(level: 2)
    db = Database(config: config, logger: logger)
    cache = Cache(config: config)
  }

  var handler: Handler {
    Handler(
      service: UserService(repo: UserRepo(db: db, logger: logger), cache: cache, logger: logger),
      config: config)
  }

  var l10: L10 {
    L10(l9: L9(l8: L8(l7: L7(l6: L6(l5: L5(l4: L4(l3: L3(l2: L2(l1: L1(l0: L0(config: config)))))))))))
  }
}

// MARK: - Injecta

@Container
final class InjectaApp: Sendable {
  let config: Config
  @Provides(.singleton) func makeLogger() -> any Logger { ConsoleLogger(level: 2) }
  @Singleton var db: Database
  @Singleton var cache: Cache
  @Transient var repo: UserRepo
  @Transient var service: UserService
  @Transient var handler: Handler
  @Transient var l0: L0
  @Transient var l1: L1
  @Transient var l2: L2
  @Transient var l3: L3
  @Transient var l4: L4
  @Transient var l5: L5
  @Transient var l6: L6
  @Transient var l7: L7
  @Transient var l8: L8
  @Transient var l9: L9
  @Transient var l10: L10
}

// MARK: - Factory 3

extension FactoryKit.Container {
  var config: Factory<Config> { self { Bench.config }.singleton }
  var logger: Factory<any Logger> { self { ConsoleLogger(level: 2) }.singleton }
  var db: Factory<Database> { self { Database(config: self.config(), logger: self.logger()) }.singleton }
  var cache: Factory<Cache> { self { Cache(config: self.config()) }.singleton }
  var repo: Factory<UserRepo> { self { UserRepo(db: self.db(), logger: self.logger()) } }
  var service: Factory<UserService> {
    self { UserService(repo: self.repo(), cache: self.cache(), logger: self.logger()) }
  }
  var handler: Factory<Handler> { self { Handler(service: self.service(), config: self.config()) } }
  var l0: Factory<L0> { self { L0(config: self.config()) } }
  var l1: Factory<L1> { self { L1(l0: self.l0()) } }
  var l2: Factory<L2> { self { L2(l1: self.l1()) } }
  var l3: Factory<L3> { self { L3(l2: self.l2()) } }
  var l4: Factory<L4> { self { L4(l3: self.l3()) } }
  var l5: Factory<L5> { self { L5(l4: self.l4()) } }
  var l6: Factory<L6> { self { L6(l5: self.l5()) } }
  var l7: Factory<L7> { self { L7(l6: self.l6()) } }
  var l8: Factory<L8> { self { L8(l7: self.l7()) } }
  var l9: Factory<L9> { self { L9(l8: self.l8()) } }
  var l10: Factory<L10> { self { L10(l9: self.l9()) } }
}

enum Bench {
  static let config = di_bench.config
}

// MARK: - Swinject 2

func swinjectContainer() -> Swinject.Container {
  let c = Swinject.Container()
  c.register(Config.self) { _ in config }.inObjectScope(.container)
  c.register((any Logger).self) { _ in ConsoleLogger(level: 2) }.inObjectScope(.container)
  c.register(Database.self) { r in
    Database(config: r.resolve(Config.self)!, logger: r.resolve((any Logger).self)!)
  }.inObjectScope(.container)
  c.register(Cache.self) { r in Cache(config: r.resolve(Config.self)!) }.inObjectScope(.container)
  c.register(UserRepo.self) { r in
    UserRepo(db: r.resolve(Database.self)!, logger: r.resolve((any Logger).self)!)
  }.inObjectScope(.transient)
  c.register(UserService.self) { r in
    UserService(repo: r.resolve(UserRepo.self)!, cache: r.resolve(Cache.self)!, logger: r.resolve((any Logger).self)!)
  }.inObjectScope(.transient)
  c.register(Handler.self) { r in
    Handler(service: r.resolve(UserService.self)!, config: r.resolve(Config.self)!)
  }.inObjectScope(.transient)
  c.register(L0.self) { r in L0(config: r.resolve(Config.self)!) }.inObjectScope(.transient)
  c.register(L1.self) { r in L1(l0: r.resolve(L0.self)!) }.inObjectScope(.transient)
  c.register(L2.self) { r in L2(l1: r.resolve(L1.self)!) }.inObjectScope(.transient)
  c.register(L3.self) { r in L3(l2: r.resolve(L2.self)!) }.inObjectScope(.transient)
  c.register(L4.self) { r in L4(l3: r.resolve(L3.self)!) }.inObjectScope(.transient)
  c.register(L5.self) { r in L5(l4: r.resolve(L4.self)!) }.inObjectScope(.transient)
  c.register(L6.self) { r in L6(l5: r.resolve(L5.self)!) }.inObjectScope(.transient)
  c.register(L7.self) { r in L7(l6: r.resolve(L6.self)!) }.inObjectScope(.transient)
  c.register(L8.self) { r in L8(l7: r.resolve(L7.self)!) }.inObjectScope(.transient)
  c.register(L9.self) { r in L9(l8: r.resolve(L8.self)!) }.inObjectScope(.transient)
  c.register(L10.self) { r in L10(l9: r.resolve(L9.self)!) }.inObjectScope(.transient)
  return c
}

// MARK: - swift-dependencies 1.17: singletons are dependency values; there are no transient
// providers, so a transient is an init call reading `@Dependency` values, as an app writes it.

private enum ConfigKey: DependencyKey { static let liveValue = config }
private enum LoggerKey: DependencyKey { static let liveValue: any Logger = ConsoleLogger(level: 2) }
private enum DatabaseKey: DependencyKey {
  static var liveValue: Database {
    @Dependency(\.benchConfig) var config
    @Dependency(\.benchLogger) var logger
    return Database(config: config, logger: logger)
  }
}
private enum CacheKey: DependencyKey {
  static var liveValue: Cache {
    @Dependency(\.benchConfig) var config
    return Cache(config: config)
  }
}

extension DependencyValues {
  var benchConfig: Config {
    get { self[ConfigKey.self] }
    set { self[ConfigKey.self] = newValue }
  }
  var benchLogger: any Logger {
    get { self[LoggerKey.self] }
    set { self[LoggerKey.self] = newValue }
  }
  var benchDatabase: Database {
    get { self[DatabaseKey.self] }
    set { self[DatabaseKey.self] = newValue }
  }
  var benchCache: Cache {
    get { self[CacheKey.self] }
    set { self[CacheKey.self] = newValue }
  }
}

enum PointFree {
  static func handler() -> Handler {
    @Dependency(\.benchConfig) var config
    @Dependency(\.benchLogger) var logger
    @Dependency(\.benchDatabase) var db
    @Dependency(\.benchCache) var cache
    return Handler(
      service: UserService(repo: UserRepo(db: db, logger: logger), cache: cache, logger: logger),
      config: config)
  }

  static func database() -> Database {
    @Dependency(\.benchDatabase) var db
    return db
  }
}
