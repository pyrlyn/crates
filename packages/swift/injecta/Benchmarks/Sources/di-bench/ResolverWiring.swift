// Its own file because Swinject also declares a `Resolver`; this file imports only one of them.

import Resolver

typealias HMResolver = Resolver

// MARK: - Resolver 1.5 (an instance, not the global `Resolver.main`, so cold starts are fair):
// `.application` is a singleton, `.unique` a new instance per resolve.

func resolverContainer() -> HMResolver {
  let r = Resolver()
  r.register { config }.scope(.application)
  r.register { ConsoleLogger(level: 2) as any Logger }.scope(.application)
  r.register { Database(config: r.resolve(), logger: r.resolve()) }.scope(.application)
  r.register { Cache(config: r.resolve()) }.scope(.application)
  r.register { UserRepo(db: r.resolve(), logger: r.resolve()) }.scope(.unique)
  r.register { UserService(repo: r.resolve(), cache: r.resolve(), logger: r.resolve()) }.scope(.unique)
  r.register { Handler(service: r.resolve(), config: r.resolve()) }.scope(.unique)
  r.register { L0(config: r.resolve()) }.scope(.unique)
  r.register { L1(l0: r.resolve()) }.scope(.unique)
  r.register { L2(l1: r.resolve()) }.scope(.unique)
  r.register { L3(l2: r.resolve()) }.scope(.unique)
  r.register { L4(l3: r.resolve()) }.scope(.unique)
  r.register { L5(l4: r.resolve()) }.scope(.unique)
  r.register { L6(l5: r.resolve()) }.scope(.unique)
  r.register { L7(l6: r.resolve()) }.scope(.unique)
  r.register { L8(l7: r.resolve()) }.scope(.unique)
  r.register { L9(l8: r.resolve()) }.scope(.unique)
  r.register { L10(l9: r.resolve()) }.scope(.unique)
  return r
}
