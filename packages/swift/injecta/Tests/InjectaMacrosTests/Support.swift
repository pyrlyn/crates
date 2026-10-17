import Foundation
import InjectaMacros
import SwiftParser
import SwiftSyntax
import SwiftSyntaxMacroExpansion
import SwiftSyntaxMacros
import XCTest

/// The macros with the conformances the compiler would ask them for.
let specs: [String: MacroSpec] = macros.mapValues { type in
  if type == InjectableMacro.self { return MacroSpec(type: type, conformances: ["Injectable"]) }
  if type == ContainerMacro.self { return MacroSpec(type: type, conformances: ["Container"]) }
  return MacroSpec(type: type)
}

/// The expanded source, for checks that only care about a few lines.
func expand(_ source: String) -> String {
  let context = BasicMacroExpansionContext()
  let tree = Parser.parse(source: source)
  return tree.expand(macroSpecs: specs, contextGenerator: { _ in context }, indentationWidth: .spaces(2))
    .description
}

/// The messages of every diagnostic the expansion produced.
func expandDiagnostics(_ source: String) -> [String] {
  let context = BasicMacroExpansionContext()
  let tree = Parser.parse(source: source)
  _ = tree.expand(macroSpecs: specs, contextGenerator: { _ in context }, indentationWidth: .spaces(2))
  return context.diagnostics.map(\.message)
}

/// Compares the expansion of `source` with `expected` with all whitespace removed: the tokens
/// and their order are the contract, the generated layout is not.
func assertExpansion(
  _ source: String, _ expected: String, file: StaticString = #filePath, line: UInt = #line
) {
  func squeezed(_ text: String) -> String { text.filter { !$0.isWhitespace } }
  let actual = expand(source)
  XCTAssertEqual(squeezed(actual), squeezed(expected), "\n" + actual, file: file, line: line)
}
