// The graph every implementation wires, the same shape as the Rust crate's comparison benchmark:
// two inputs, three singletons, three transients, plus a ten-level transient chain.

import Injecta

struct Config: Sendable {
  let poolSize: Int
  let timeoutMs: Int
}

protocol Logger: Sendable { var level: Int { get } }
struct ConsoleLogger: Logger { let level: Int }

@Injectable
final class Database: Sendable {
  let config: Config
  let logger: any Logger
  init(config: Config, logger: any Logger) { (self.config, self.logger) = (config, logger) }
}

@Injectable
final class Cache: Sendable {
  let capacity: Int
  init(config: Config) { capacity = config.poolSize * 16 }
}

@Injectable
struct UserRepo: Sendable {
  let db: Database
  let logger: any Logger
}

@Injectable
struct UserService: Sendable {
  let repo: UserRepo
  let cache: Cache
  let logger: any Logger
}

@Injectable
struct Handler: Sendable {
  let service: UserService
  let config: Config

  var checksum: Int {
    config.poolSize + config.timeoutMs + service.cache.capacity + service.repo.db.config.poolSize
      + service.logger.level + service.repo.logger.level
  }
}

// A ten-level chain of transients: L10 needs L9 ... L1 needs L0.
@Injectable struct L0: Sendable { let config: Config }
@Injectable struct L1: Sendable { let l0: L0 }
@Injectable struct L2: Sendable { let l1: L1 }
@Injectable struct L3: Sendable { let l2: L2 }
@Injectable struct L4: Sendable { let l3: L3 }
@Injectable struct L5: Sendable { let l4: L4 }
@Injectable struct L6: Sendable { let l5: L5 }
@Injectable struct L7: Sendable { let l6: L6 }
@Injectable struct L8: Sendable { let l7: L7 }
@Injectable struct L9: Sendable { let l8: L8 }
@Injectable struct L10: Sendable {
  let l9: L9
  var depth: Int { l9.l8.l7.l6.l5.l4.l3.l2.l1.l0.config.poolSize }
}

let config = Config(poolSize: 8, timeoutMs: 500)
