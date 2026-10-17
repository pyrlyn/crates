// A target with no InjectaCheckPlugin. A hand-written `Needs` line still resolves. A missing
// one compiles in a debug build and traps on init, naming the plugin. Release builds fail at
// compile time instead, so the broken container is compiled only in debug.

import Injecta
import Testing

@Test func theMessageNamesThePluginAndTheConformance() {
  let text = missingPluginMessage("SettingsStore")
  #expect(text.contains("does not conform to SettingsStore.Needs"))
  #expect(text.contains("InjectaCheckPlugin"))
  #expect(text.contains("extension <Container>: SettingsStore.Needs {}"))
}

#if DEBUG
  @Injectable
  struct Leaf {
    let value: Int
  }

  @Container
  final class Broken: Sendable {
    let value: Int
    @Singleton var leaf: Leaf
  }

  @Container
  final class HandWired: Sendable {
    let value: Int
    @Singleton var leaf: Leaf
  }

  extension HandWired: Leaf.Needs {}

  @Test func aHandWrittenConformanceResolvesWithoutThePlugin() {
    #expect(HandWired(value: 4).leaf.value == 4)
    #expect(HandWired.injectaGraph.issues().isEmpty)
  }

  @Test func aMissingConformanceTrapsAndNamesThePlugin() async {
    let result = await #expect(processExitsWith: .failure, observing: [\.standardErrorContent]) {
      _ = Broken(value: 1)
    }
    let stderr = String(decoding: result?.standardErrorContent ?? [], as: UTF8.self)
    #expect(stderr.contains(missingPluginMessage("Leaf")))
  }
#endif
