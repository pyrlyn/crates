import SwiftCompilerPlugin
import SwiftSyntaxMacros

@main
struct InjectaPlugin: CompilerPlugin {
  let providingMacros: [Macro.Type] = [
    InjectableMacro.self,
    InjectMacro.self,
    ContainerMacro.self,
    SingletonMacro.self,
    TransientMacro.self,
    ForwardMacro.self,
    ProvidesMacro.self,
  ]
}
