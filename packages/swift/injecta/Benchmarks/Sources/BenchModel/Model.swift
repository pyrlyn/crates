// The comparison graph as plain types, for the `size-*` executables: each wires it with one
// library, so the binaries differ only by that library and its wiring.

public struct Config: Sendable {
  public let poolSize: Int
  public let timeoutMs: Int
  public init(poolSize: Int, timeoutMs: Int) { (self.poolSize, self.timeoutMs) = (poolSize, timeoutMs) }
}

public protocol Logger: Sendable { var level: Int { get } }
public struct ConsoleLogger: Logger {
  public let level: Int
  public init(level: Int) { self.level = level }
}

public final class Database: Sendable {
  public let config: Config
  public let logger: any Logger
  public init(config: Config, logger: any Logger) { (self.config, self.logger) = (config, logger) }
}

public final class Cache: Sendable {
  public let capacity: Int
  public init(config: Config) { capacity = config.poolSize * 16 }
}

public struct UserRepo: Sendable {
  public let db: Database
  public let logger: any Logger
  public init(db: Database, logger: any Logger) { (self.db, self.logger) = (db, logger) }
}

public struct UserService: Sendable {
  public let repo: UserRepo
  public let cache: Cache
  public let logger: any Logger
  public init(repo: UserRepo, cache: Cache, logger: any Logger) {
    (self.repo, self.cache, self.logger) = (repo, cache, logger)
  }
}

public struct Handler: Sendable {
  public let service: UserService
  public let config: Config
  public init(service: UserService, config: Config) { (self.service, self.config) = (service, config) }
  public var checksum: Int {
    config.poolSize + config.timeoutMs + service.cache.capacity + service.repo.db.config.poolSize
      + service.logger.level + service.repo.logger.level
  }
}

public let benchConfig = Config(poolSize: 8, timeoutMs: 500)
