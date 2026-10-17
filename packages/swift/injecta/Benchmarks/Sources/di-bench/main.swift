// di-bench: nanoseconds per operation, median of 15 runs, release build. Every operation feeds a
// checksum into a sink so the optimizer cannot drop it. Run through `measure.sh`.

import Dependencies
import Dispatch
import Foundation
import FactoryKit
import Injecta
import Resolver
import Swinject

nonisolated(unsafe) var sink = 0

@inline(never)
func consume(_ value: Int) { sink &+= value }

/// Median nanoseconds per operation of `body(iterations)` over `runs` runs, after one warm-up.
func measure(_ iterations: Int, runs: Int = 15, _ body: (Int) -> Void) -> Double {
  body(iterations / 10)
  var samples: [Double] = []
  for _ in 0..<runs {
    let start = ContinuousClock.now
    body(iterations)
    let elapsed = ContinuousClock.now - start
    let (seconds, attoseconds) = elapsed.components
    samples.append((Double(seconds) * 1e9 + Double(attoseconds) / 1e9) / Double(iterations))
  }
  samples.sort()
  return samples[samples.count / 2]
}

// One sink per thread, a cache line apart, so threads do not contend on the sink itself.
let lanes = UnsafeMutablePointer<Int>.allocate(capacity: 64 * 16)
lanes.initialize(repeating: 0, count: 64 * 16)

@inline(never)
func consume(_ value: Int, lane: Int) { lanes[lane * 16] &+= value }

/// The same, with `threads` threads each running `body(iterations, lane)` at once; per operation.
func measureConcurrent(
  _ iterations: Int, threads: Int, runs: Int = 9, _ body: @Sendable (Int, Int) -> Void
) -> Double {
  var samples: [Double] = []
  for _ in 0..<runs {
    let start = ContinuousClock.now
    DispatchQueue.concurrentPerform(iterations: threads) { lane in body(iterations, lane) }
    let elapsed = ContinuousClock.now - start
    let (seconds, attoseconds) = elapsed.components
    samples.append((Double(seconds) * 1e9 + Double(attoseconds) / 1e9) / Double(iterations * threads))
  }
  samples.sort()
  return samples[samples.count / 2]
}

let n = 1_000_000
let cold = 20_000

let manual = Manual(config: config)
let injecta = InjectaApp(config: config)
let factory = FactoryKit.Container()
let swinject = swinjectContainer()
let swinjectShared = swinject.synchronize()
let resolver = resolverContainer()

var rows: [(String, [String: Double])] = []

func row(_ name: String, _ cells: [String: Double]) { rows.append((name, cells)) }

row(
  "singleton (cached)",
  [
    "manual": measure(n) { for _ in 0..<$0 { consume(manual.db.config.poolSize) } },
    "injecta": measure(n) { for _ in 0..<$0 { consume(injecta.db.config.poolSize) } },
    "swift-dependencies": measure(n) { for _ in 0..<$0 { consume(PointFree.database().config.poolSize) } },
    "Factory": measure(n) { for _ in 0..<$0 { consume(factory.db().config.poolSize) } },
    "Swinject": measure(n / 10) { for _ in 0..<$0 { consume(swinject.resolve(Database.self)!.config.poolSize) } },
    "Resolver": measure(n / 10) { for _ in 0..<$0 { consume((resolver.resolve() as Database).config.poolSize) } },
  ])

row(
  "graph (3 transients + 3 singletons)",
  [
    "manual": measure(n) { for _ in 0..<$0 { consume(manual.handler.checksum) } },
    "injecta": measure(n) { for _ in 0..<$0 { consume(injecta.handler.checksum) } },
    "swift-dependencies": measure(n) { for _ in 0..<$0 { consume(PointFree.handler().checksum) } },
    "Factory": measure(n / 10) { for _ in 0..<$0 { consume(factory.handler().checksum) } },
    "Swinject": measure(n / 100) { for _ in 0..<$0 { consume(swinject.resolve(Handler.self)!.checksum) } },
    "Resolver": measure(n / 100) { for _ in 0..<$0 { consume((resolver.resolve() as Handler).checksum) } },
  ])

row(
  "chain of 10 transients",
  [
    "manual": measure(n) { for _ in 0..<$0 { consume(manual.l10.depth) } },
    "injecta": measure(n) { for _ in 0..<$0 { consume(injecta.l10.depth) } },
    "Factory": measure(n / 10) { for _ in 0..<$0 { consume(factory.l10().depth) } },
    "Swinject": measure(n / 100) { for _ in 0..<$0 { consume(swinject.resolve(L10.self)!.depth) } },
    "Resolver": measure(n / 100) { for _ in 0..<$0 { consume((resolver.resolve() as L10).depth) } },
  ])

row(
  "cold start (new container + first graph)",
  [
    "manual": measure(cold) { for _ in 0..<$0 { consume(Manual(config: config).handler.checksum) } },
    "injecta": measure(cold) { for _ in 0..<$0 { consume(InjectaApp(config: config).handler.checksum) } },
    "Factory": measure(cold) { for _ in 0..<$0 { consume(FactoryKit.Container().handler().checksum) } },
    "Swinject": measure(cold / 10) { for _ in 0..<$0 { consume(swinjectContainer().resolve(Handler.self)!.checksum) } },
    "Resolver": measure(cold / 10) { for _ in 0..<$0 { consume((resolverContainer().resolve() as Handler).checksum) } },
  ])

row(
  "override scope + graph",
  [
    "injecta": measure(cold) { iterations in
      for _ in 0..<iterations {
        var overrides = InjectaApp.Overrides()
        overrides.logger = ConsoleLogger(level: 1)
        consume(InjectaApp(config: config, overrides: overrides).handler.checksum)
      }
    },
    "swift-dependencies": measure(cold) { iterations in
      for _ in 0..<iterations {
        consume(withDependencies { $0.benchLogger = ConsoleLogger(level: 1) } operation: { PointFree.handler().checksum })
      }
    },
  ])

let threads = 8
row(
  "graph, \(threads) threads at once (per op)",
  [
    "manual": measureConcurrent(n / 10, threads: threads) { for _ in 0..<$0 { consume(manual.handler.checksum, lane: $1) } },
    "injecta": measureConcurrent(n / 10, threads: threads) { for _ in 0..<$0 { consume(injecta.handler.checksum, lane: $1) } },
    "swift-dependencies": measureConcurrent(n / 10, threads: threads) { for _ in 0..<$0 { consume(PointFree.handler().checksum, lane: $1) } },
    "Factory": measureConcurrent(n / 100, threads: threads) { for _ in 0..<$0 { consume(factory.handler().checksum, lane: $1) } },
    "Swinject (synchronized)": measureConcurrent(n / 1000, threads: threads) { for _ in 0..<$0 { consume(swinjectShared.resolve(Handler.self)!.checksum, lane: $1) } },
    "Resolver": measureConcurrent(n / 1000, threads: threads) { for _ in 0..<$0 { consume((resolver.resolve() as Handler).checksum, lane: $1) } },
  ])

func cell(_ value: Double?) -> String {
  guard let value else { return "n/a" }
  return value < 100 ? String(format: "%.1f ns", value) : String(format: "%.0f ns", value)
}
// The concurrent Swinject case uses `synchronize()`, the thread-safe resolver Swinject documents.
print("| case | manual | injecta | swift-dependencies | Factory | Swinject | Resolver |")
print("| --- | ---: | ---: | ---: | ---: | ---: | ---: |")
for (name, cells) in rows {
  let swinjectCell = cells["Swinject"] ?? cells["Swinject (synchronized)"]
  let values = [cells["manual"], cells["injecta"], cells["swift-dependencies"], cells["Factory"], swinjectCell, cells["Resolver"]].map(cell)
  print("| \(name) | " + values.joined(separator: " | ") + " |")
}
print("checksum sink: \(sink &+ (0..<64).reduce(0) { $0 &+ lanes[$1 * 16] })")
