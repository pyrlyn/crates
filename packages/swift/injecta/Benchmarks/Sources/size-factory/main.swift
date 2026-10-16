import BenchModel
import FactoryKit

extension Container {
  var config: Factory<Config> { self { benchConfig }.singleton }
  var logger: Factory<any Logger> { self { ConsoleLogger(level: 2) }.singleton }
  var db: Factory<Database> { self { Database(config: self.config(), logger: self.logger()) }.singleton }
  var cache: Factory<Cache> { self { Cache(config: self.config()) }.singleton }
  var repo: Factory<UserRepo> { self { UserRepo(db: self.db(), logger: self.logger()) } }
  var service: Factory<UserService> {
    self { UserService(repo: self.repo(), cache: self.cache(), logger: self.logger()) }
  }
  var handler: Factory<Handler> { self { Handler(service: self.service(), config: self.config()) } }
}

print(Container.shared.handler().checksum)
