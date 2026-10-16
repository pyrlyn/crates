import BenchModel

let logger: any Logger = ConsoleLogger(level: 2)
let db = Database(config: benchConfig, logger: logger)
let cache = Cache(config: benchConfig)
let handler = Handler(
  service: UserService(repo: UserRepo(db: db, logger: logger), cache: cache, logger: logger),
  config: benchConfig)
print(handler.checksum)
