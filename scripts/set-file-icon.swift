// Apply the approved ICNS as Finder metadata on a packaging artifact only.
// HTTP downloads discard this metadata; package-dmg.mjs also creates a ZIP
// that preserves it, rather than claiming a plain DMG download retains it.
import AppKit
import Foundation

guard CommandLine.arguments.count == 3,
      let icon = NSImage(contentsOfFile: CommandLine.arguments[1]),
      NSWorkspace.shared.setIcon(icon, forFile: CommandLine.arguments[2], options: []) else {
    fputs("Could not set the installer file icon.\n", stderr)
    exit(1)
}
