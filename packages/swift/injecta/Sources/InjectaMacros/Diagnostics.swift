import Foundation
import SwiftDiagnostics
import SwiftSyntax
import SwiftSyntaxMacros

struct InjectaMessage: DiagnosticMessage {
  let message: String
  let severity: DiagnosticSeverity
  var diagnosticID: MessageID { MessageID(domain: "Injecta", id: severity == .error ? "error" : "warning") }
}

extension MacroExpansionContext {
  /// Every problem at once, so one build shows all of them.
  func report(_ problems: [Problem]) {
    for problem in problems {
      diagnose(
        Diagnostic(
          node: problem.node,
          message: InjectaMessage(
            message: problem.message, severity: problem.severity == .error ? .error : .warning)))
    }
  }
}

/// A type as a protocol requirement or stored type: `some P` cannot be either, and `@escaping`
/// belongs to parameters only.
func storable(_ type: String) -> String {
  var text = type.replacingFirst("@escaping ", with: "")
  if text.hasPrefix("some ") { text = "any " + text.dropFirst(5) }
  return text
}

extension String {
  func replacingFirst(_ target: String, with replacement: String) -> String {
    guard let range = range(of: target) else { return self }
    return replacingCharacters(in: range, with: replacement)
  }
}
