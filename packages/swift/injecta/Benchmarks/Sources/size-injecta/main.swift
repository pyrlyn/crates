import BenchModel
import Injecta

// Providers only: these types live in another module, so they cannot be @Injectable here.
@Container
final class App: Sendable {
  let config: Config
  @Provides(.singleton) func makeLogger() -> any Logger { ConsoleLogger(level: 2) }
  @Provides(.singleton) func makeDb(config: Config, logger: any Logger) -> Database {
    Database(config: config, logger: logger)
  }
  @Provides(.singleton) func makeCache(config: Config) -> Cache { Cache(config: config) }
  @Provides(.transient) func makeRepo(db: Database, logger: any Logger) -> UserRepo {
    UserRepo(db: db, logger: logger)
  }
  @Provides(.transient) func makeService(repo: UserRepo, cache: Cache, logger: any Logger) -> UserService {
    UserService(repo: repo, cache: cache, logger: logger)
  }
  @Provides(.transient) func makeHandler(service: UserService, config: Config) -> Handler {
    Handler(service: service, config: config)
  }
}

print(App(config: benchConfig).handler.checksum)
