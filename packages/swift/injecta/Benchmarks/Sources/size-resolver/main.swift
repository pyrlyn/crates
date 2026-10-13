import BenchModel
import Resolver

let r = Resolver()
r.register { benchConfig }.scope(.application)
r.register { ConsoleLogger(level: 2) as any Logger }.scope(.application)
r.register { Database(config: r.resolve(), logger: r.resolve()) }.scope(.application)
r.register { Cache(config: r.resolve()) }.scope(.application)
r.register { UserRepo(db: r.resolve(), logger: r.resolve()) }.scope(.unique)
r.register { UserService(repo: r.resolve(), cache: r.resolve(), logger: r.resolve()) }.scope(.unique)
r.register { Handler(service: r.resolve(), config: r.resolve()) }.scope(.unique)
print((r.resolve() as Handler).checksum)
