import BenchModel
import Swinject

let c = Container()
c.register(Config.self) { _ in benchConfig }.inObjectScope(.container)
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
print(c.resolve(Handler.self)!.checksum)
