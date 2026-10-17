import BenchModel
import Dependencies

private enum ConfigKey: DependencyKey { static let liveValue = benchConfig }
private enum LoggerKey: DependencyKey { static let liveValue: any Logger = ConsoleLogger(level: 2) }
private enum DatabaseKey: DependencyKey {
  static var liveValue: Database {
    @Dependency(\.config) var config
    @Dependency(\.logger) var logger
    return Database(config: config, logger: logger)
  }
}
private enum CacheKey: DependencyKey {
  static var liveValue: Cache {
    @Dependency(\.config) var config
    return Cache(config: config)
  }
}

extension DependencyValues {
  var config: Config {
    get { self[ConfigKey.self] }
    set { self[ConfigKey.self] = newValue }
  }
  var logger: any Logger {
    get { self[LoggerKey.self] }
    set { self[LoggerKey.self] = newValue }
  }
  var db: Database {
    get { self[DatabaseKey.self] }
    set { self[DatabaseKey.self] = newValue }
  }
  var cache: Cache {
    get { self[CacheKey.self] }
    set { self[CacheKey.self] = newValue }
  }
}

func handler() -> Handler {
  @Dependency(\.config) var config
  @Dependency(\.logger) var logger
  @Dependency(\.db) var db
  @Dependency(\.cache) var cache
  return Handler(
    service: UserService(repo: UserRepo(db: db, logger: logger), cache: cache, logger: logger),
    config: config)
}

print(handler().checksum)
