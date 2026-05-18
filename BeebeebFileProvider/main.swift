import Foundation

@_silgen_name("NSExtensionMain")
func NSExtensionMain(
    _ argc: Int32,
    _ argv: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>
) -> Int32

let exitCode = NSExtensionMain(CommandLine.argc, CommandLine.unsafeArgv)
exit(exitCode)
