// Runs `injecta-check` over a target's Swift files before it compiles, so a missing entry or a
// cycle that crosses files fails the build with the file and line, not at run time.

import Foundation
import PackagePlugin

@main
struct InjectaCheckPlugin: BuildToolPlugin {
  func createBuildCommands(context: PluginContext, target: Target) throws -> [Command] {
    guard let target = target as? SourceModuleTarget else { return [] }
    let files = target.sourceFiles(withSuffix: "swift").map(\.url)
    return [try command(tool: context.tool(named: "injecta-check").url, work: context.pluginWorkDirectoryURL, name: target.name, files: files)]
  }
}

func command(tool: URL, work: URL, name: String, files: [URL]) -> Command {
  let output = work.appending(path: "InjectaGraphCheck_\(name).swift")
  return .buildCommand(
    displayName: "Injecta graph check (\(name))",
    executable: tool,
    arguments: [output.path(percentEncoded: false)] + files.map { $0.path(percentEncoded: false) },
    inputFiles: files,
    outputFiles: [output])
}

#if canImport(XcodeProjectPlugin)
  import XcodeProjectPlugin

  extension InjectaCheckPlugin: XcodeBuildToolPlugin {
    func createBuildCommands(context: XcodePluginContext, target: XcodeTarget) throws -> [Command] {
      let files = target.inputFiles.filter { $0.url.pathExtension == "swift" }.map(\.url)
      return [try command(tool: context.tool(named: "injecta-check").url, work: context.pluginWorkDirectoryURL, name: target.displayName, files: files)]
    }
  }
#endif
