// `llms.txt` and `llms-full.txt` are derived from GUIDE.md and AGENTS.md; this test regenerates
// them with INJECTA_BLESS=1 and otherwise fails when they are stale, so the docs cannot drift.

import Foundation
import Testing

private let root = URL(filePath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
  .deletingLastPathComponent()

private func read(_ name: String) throws -> String {
  try String(contentsOf: root.appending(path: name), encoding: .utf8)
}

private let summary = """
  > Compile-time dependency injection for Swift 6. Declare a container once with `@Container`, \
  mark the types it builds `@Injectable`, and read entries as properties. The macros write the \
  wiring as plain initializer calls; a build-tool plugin checks the whole graph across files. A \
  missing entry, a cycle or a misuse is a compile error at its line. A singleton read is one \
  atomic load: no registry, dictionary, lock or task-local on the resolve path.
  """

private func llms(guide: String) -> String {
  let sections = guide.split(separator: "\n").filter { $0.hasPrefix("## ") }.map { "- " + $0.dropFirst(3) }
  return """
    # Injecta (Swift)

    \(summary)

    Generated from GUIDE.md and AGENTS.md by `INJECTA_BLESS=1 swift test --filter DocsTests`; do not edit.

    ## Docs

    - [Guide](GUIDE.md): the user guide
    - [Full text](llms-full.txt): the guide and the agent rules in one file
    - [Agent rules](AGENTS.md): how to use and how to change Injecta
    - [Example container](Tests/InjectaTests/Fixtures.swift): app graph, request scope, main-actor graph

    ## Guide sections

    \(sections.joined(separator: "\n"))

    """
}

@Suite("Docs")
struct DocsTests {
  @Test func llmsFilesAreGeneratedFromTheGuideAndTheAgentRules() throws {
    let guide = try read("GUIDE.md")
    let agents = try read("AGENTS.md")
    let wanted = [
      "llms.txt": llms(guide: guide),
      "llms-full.txt": "# Injecta (Swift): full documentation\n\n\(summary)\n\n" + guide + "\n" + agents,
    ]
    for (name, text) in wanted {
      if ProcessInfo.processInfo.environment["INJECTA_BLESS"] == "1" {
        try text.write(to: root.appending(path: name), atomically: true, encoding: .utf8)
      } else {
        #expect(try read(name) == text, "\(name) is stale: run INJECTA_BLESS=1 swift test --filter DocsTests")
      }
    }
  }
}
