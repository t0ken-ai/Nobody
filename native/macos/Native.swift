// Native macOS adapter. All AppKit / AX access is serialized on the main queue.
// Rust owns the request lifetime; callbacks copy the JSON before this stack ends.
// Own-window title lookups below must match tauri.conf.json's Nobody titles.
// Rebranding just the config would silently break styling, cleanup and popovers.
import AppKit
import ApplicationServices
import Foundation
import Darwin
import NaturalLanguage
import UniformTypeIdentifiers
import OSLog
#if canImport(Translation)
import SwiftUI
import Translation
#endif

typealias Reply = @convention(c) (UnsafePointer<CChar>?, UnsafeMutableRawPointer?) -> Void

private func respond(_ value: [String: Any], _ reply: Reply, _ context: UnsafeMutableRawPointer?) {
    let data = (try? JSONSerialization.data(withJSONObject: value)) ?? Data("{\"error\":\"Native encoding failed\"}".utf8)
    String(decoding: data, as: UTF8.self).withCString { reply($0, context) }
}

private func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success ? value : nil
}

private func textAttribute(_ element: AXUIElement, _ name: String) -> String? { attribute(element, name) as? String }
private func parameterizedAttribute(_ element: AXUIElement, _ name: String, _ parameter: CFTypeRef) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyParameterizedAttributeValue(element, name as CFString, parameter, &value) == .success ? value : nil
}
private func isSettable(_ element: AXUIElement, _ name: String) -> Bool {
    var writable = DarwinBoolean(false)
    return AXUIElementIsAttributeSettable(element, name as CFString, &writable) == .success && writable.boolValue
}
private func rangeAttribute(_ element: AXUIElement) -> CFRange? {
    guard let raw = attribute(element, kAXSelectedTextRangeAttribute), CFGetTypeID(raw) == AXValueGetTypeID() else { return nil }
    var range = CFRange()
    return AXValueGetValue(unsafeBitCast(raw, to: AXValue.self), .cfRange, &range) ? range : nil
}

private struct Snapshot {
    let element: AXUIElement
    let pid: pid_t
    let value: String?
    let selected: String
    let range: CFRange?
    let whole: Bool
    let created: Date
}
private var snapshots: [String: Snapshot] = [:]
private var backgroundCleanup: DispatchWorkItem?

/// Startup/IPC can leave freed allocations in malloc's caches. After five
/// seconds hidden, return only unused pages to macOS; live drafts, keys and
/// transfers remain allocated. This is one-shot, never a polling trim loop.
private func scheduleBackgroundCleanup() {
    backgroundCleanup?.cancel()
    let work = DispatchWorkItem {
        backgroundCleanup = nil
        guard let main = NSApp.windows.first(where: { $0.title == "Nobody" }),
              !main.isVisible || main.isMiniaturized else { return }
        // The system allocator decides what can be unmapped safely. Do not
        // force page-out or touch active allocations to lower a memory counter.
        malloc_zone_pressure_relief(nil, 0)
    }
    backgroundCleanup = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 5, execute: work)
}

/// Presets identify the official clients, not similarly named wrappers. A
/// custom selection additionally binds to its canonical app-bundle location.
private let presetBundles = ["chatgpt": "com.openai.chat", "claude": "com.anthropic.claudefordesktop"]
private func applicationAllowed(_ app: NSRunningApplication, _ allowed: [Any]) -> Bool {
    allowed.contains { entry in
        if let preset = entry as? String {
            return presetBundles[preset].map { $0 == app.bundleIdentifier } ?? false
        }
        guard let local = entry as? [String: Any], local["platform"] as? String == "macOS",
              let path = local["path"] as? String, let bundle = local["bundleId"] as? String,
              let url = app.bundleURL else { return false }
        return bundle == app.bundleIdentifier && url.resolvingSymlinksInPath().standardizedFileURL.path == path
    }
}

/// Resolve a chosen local bundle without launching it or reading its documents.
/// Aliases/symlinks are normalized once; network volumes and ordinary folders
/// are rejected even if a caller bypasses the Open panel's extension filter.
private func applicationDescriptor(_ selected: URL) throws -> [String: Any] {
    let values = try selected.resourceValues(forKeys: [.isAliasFileKey])
    let resolved = values.isAliasFile == true
        ? try URL(resolvingAliasFileAt: selected, options: [.withoutUI, .withoutMounting]) : selected
    let url = resolved.resolvingSymlinksInPath().standardizedFileURL
    let properties = try url.resourceValues(forKeys: [.volumeIsLocalKey, .isDirectoryKey])
    guard url.isFileURL, properties.volumeIsLocal == true, properties.isDirectory == true,
          url.pathExtension.lowercased() == "app", let bundle = Bundle(url: url),
          let identity = bundle.bundleIdentifier, !identity.isEmpty,
          bundle.object(forInfoDictionaryKey: "CFBundlePackageType") as? String == "APPL",
          let executable = bundle.executableURL, FileManager.default.isExecutableFile(atPath: executable.path) else {
        throw NativeError.message("请选择本机已安装的 .app 应用。")
    }
    // Use the installed bundle's filename: some products retain another app's
    // CFBundleName internally (e.g. Codex), which would mislabel the allowlist.
    return ["name": url.deletingPathExtension().lastPathComponent, "path": url.path,
            "platform": "macOS", "bundleId": identity]
}

/// Missing defaults remain visible and can begin working after installation.
/// Lookup verifies the bundle ID, so ChatGPT-named third-party apps stay out.
private func defaultApplications() -> [String: Any] {
    var result: [String: Any] = [:]
    for (key, identity) in presetBundles {
        if let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: identity),
           let app = try? applicationDescriptor(url), app["bundleId"] as? String == identity {
            result[key] = app
        } else { result[key] = NSNull() }
    }
    return result
}

/// Read Finder's installed app icon without launching the app. Render to a
/// bounded 64 px PNG (32 pt at Retina scale), never send bundle files to the UI.
/// Missing/replaced apps return null individually; identity checks remain the
/// same as the picker and no icon data is written to the allowlist settings.
private func applicationIcons(_ entries: [Any]) -> [Any] {
    let defaults = defaultApplications()
    return entries.prefix(100).map { entry -> Any in
        let local = (entry as? String).flatMap { defaults[$0] as? [String: Any] }
            ?? entry as? [String: Any]
        guard let local, local["platform"] as? String == "macOS",
              let path = local["path"] as? String,
              let actual = try? applicationDescriptor(URL(fileURLWithPath: path)),
              actual["path"] as? String == path,
              actual["bundleId"] as? String == local["bundleId"] as? String,
              let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 64,
                  pixelsHigh: 64, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
                  isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
              let context = NSGraphicsContext(bitmapImageRep: bitmap) else { return NSNull() }
        let image = NSWorkspace.shared.icon(forFile: path)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = context
        image.draw(in: NSRect(x: 0, y: 0, width: 64, height: 64), from: .zero,
                   operation: .copy, fraction: 1)
        NSGraphicsContext.restoreGraphicsState()
        guard let data = bitmap.representation(using: .png, properties: [:]),
              data.count <= 65536 else { return NSNull() }
        return "data:image/png;base64," + data.base64EncodedString()
    }
}

/// Recheck picker output before saving. Never silently accept a replacement app
/// at the same path with a different identity. Presets need not be installed.
private func validateApplications(_ entries: [Any]) throws {
    for entry in entries {
        if let preset = entry as? String, presetBundles[preset] != nil { continue }
        guard let local = entry as? [String: Any], local["platform"] as? String == "macOS",
              let path = local["path"] as? String else { throw NativeError.message("请选择这台 Mac 上的应用。") }
        let actual = try applicationDescriptor(URL(fileURLWithPath: path))
        guard actual["path"] as? String == path, actual["bundleId"] as? String == local["bundleId"] as? String else {
            throw NativeError.message("应用的位置或身份已改变，请移除后重新添加。")
        }
    }
}

/// The asynchronous panel leaves selection polling and the main run loop free.
/// Cancellation returns an empty list and never modifies saved preferences.
private func pickApplications(_ reply: @escaping Reply, _ context: UnsafeMutableRawPointer?) {
    let panel = NSOpenPanel()
    panel.title = "添加可自动翻译的应用"
    panel.prompt = "添加应用"
    panel.message = "选择本机应用；只在这些应用的阅读正文中自动划词翻译。"
    panel.directoryURL = URL(fileURLWithPath: "/Applications", isDirectory: true)
    panel.allowedContentTypes = [.applicationBundle]
    panel.allowsOtherFileTypes = false
    panel.canChooseFiles = true
    panel.canChooseDirectories = false
    panel.allowsMultipleSelection = true
    panel.canCreateDirectories = false
    panel.treatsFilePackagesAsDirectories = false
    let completion: (NSApplication.ModalResponse) -> Void = { result in
        guard result == .OK else { respond(["apps": []], reply, context); return }
        do {
            let apps = try panel.urls.map { try applicationDescriptor($0) }
            respond(["apps": apps], reply, context)
        } catch NativeError.message(let message) {
            // This completion runs outside the request dispatcher's catch. A
            // Go To path can bypass the panel filter; keep our actionable error
            // instead of Foundation's opaque "NativeError error 0" fallback.
            respond(["error": message], reply, context)
        } catch { respond(["error": error.localizedDescription], reply, context) }
    }
    if let window = NSApp.keyWindow {
        panel.beginSheetModal(for: window, completionHandler: completion)
    } else { panel.begin(completionHandler: completion) }
}

/// Only mouse gesture metadata is retained, in memory. No keyboard events or
/// text are observed here. A fresh drag/double-click release is what separates
/// intentional reading from restored ranges and programmatic select-all.
private final class SelectionMouse {
    var monitor: Any?
    var allowed: [Any] = []
    var down: (pid: pid_t, point: NSPoint, double: Bool)?
    var dragged = false
    var sequence: UInt64 = 0
    var released: (pid: pid_t, time: TimeInterval)?
    var preparedPID: pid_t?

    /// Electron may build its AX tree asynchronously after activation. Prime a
    /// newly encountered allowed app once, before its first selection, without
    /// reading any focused control or document during idle metadata polling.
    func prepareAccessibility(_ app: NSRunningApplication) {
        guard preparedPID != app.processIdentifier, applicationAllowed(app, allowed), AXIsProcessTrusted() else { return }
        let root = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 0.25)
        AXUIElementSetAttributeValue(root, "AXManualAccessibility" as CFString, kCFBooleanTrue)
        preparedPID = app.processIdentifier
    }

    func configure(_ applications: [Any]) {
        allowed = applications
        // No allowlist means no automatic gestures. Remove the global monitor
        // rather than keep receiving events after the user disables the feature.
        if allowed.isEmpty {
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
            down = nil
            released = nil
            preparedPID = nil
            return
        }
        guard monitor == nil, !allowed.isEmpty else { return }
        monitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .leftMouseDragged, .leftMouseUp]) { [weak self] event in
            guard let self else { return }
            guard let app = NSWorkspace.shared.frontmostApplication,
                  applicationAllowed(app, self.allowed) else {
                self.down = nil
                return
            }
            let point = NSEvent.mouseLocation
            switch event.type {
            case .leftMouseDown:
                self.down = (app.processIdentifier, point, event.clickCount >= 2)
                self.dragged = false
            case .leftMouseDragged:
                if let start = self.down {
                    // Four logical pixels reject hand jitter on an ordinary
                    // focus click without requiring a long text selection.
                    self.dragged = self.dragged || hypot(point.x - start.point.x, point.y - start.point.y) >= 4
                }
            case .leftMouseUp:
                if let start = self.down, start.pid == app.processIdentifier,
                   self.dragged || start.double {
                    self.sequence &+= 1
                    self.released = (app.processIdentifier, ProcessInfo.processInfo.systemUptime)
                }
                self.down = nil
            default: break
            }
        }
    }

    func metadata(_ pid: pid_t) -> [String: Any] {
        guard let released, released.pid == pid else { return [:] }
        return ["gestureId": "\(pid):\(sequence)",
                "gestureAgeMs": max(0, Int((ProcessInfo.processInfo.systemUptime - released.time) * 1000))]
    }
}
private let selectionMouse = SelectionMouse()

/// Cheap metadata decides whether the coordinator needs an AX read this tick.
/// Missing hints preserve explicit/older callers; a pending debounce or a live
/// result still reads every tick so scrolling, edits and focus loss stay visible.
private func needsSelectionCapture(_ request: [String: Any], _ metadata: [String: Any], _ pid: pid_t) -> Bool {
    guard request["op"] as? String == "selection", let poll = request["poll"] as? [String: Any] else { return true }
    let tracking = request["tracking"] as? [String: Any] ?? [:]
    if tracking["pid"] as? Int32 == pid || poll["pending"] as? Bool == true { return true }
    guard metadata["mouseDown"] as? Bool != true,
          let gesture = metadata["gestureId"] as? String, !gesture.isEmpty,
          gesture != poll["gestureId"] as? String,
          let age = metadata["gestureAgeMs"] as? Int, age <= 2000 else { return false }
    return true
}

private func elementAttribute(_ element: AXUIElement, _ name: String) -> AXUIElement? {
    guard let raw = attribute(element, name), CFGetTypeID(raw) == AXUIElementGetTypeID() else { return nil }
    return unsafeBitCast(raw, to: AXUIElement.self)
}

/// Inspect only the focused element's ancestor chain. Modal file panels and
/// editable ancestors are excluded before selected text is read. Being able to
/// set a selection range is NOT evidence that the text itself is writable.
private func automaticContext(_ element: AXUIElement) -> String {
    var current: AXUIElement? = element
    var readable = false
    var visited = Set<CFHashCode>()
    for _ in 0..<24 {
        guard let node = current, visited.insert(CFHash(node)).inserted else { break }
        let role = textAttribute(node, kAXRoleAttribute) ?? ""
        let subrole = textAttribute(node, kAXSubroleAttribute) ?? ""
        if role == kAXSheetRole || subrole == kAXDialogSubrole || subrole == kAXSystemDialogSubrole
            || (attribute(node, kAXModalAttribute) as? Bool) == true { return "dialog" }
        if role == kAXTextFieldRole || role == kAXComboBoxRole
            || elementAttribute(node, "AXEditableAncestor") != nil
            || ([kAXTextAreaRole, kAXStaticTextRole].contains(role)
                && (isSettable(node, kAXValueAttribute) || isSettable(node, kAXSelectedTextAttribute))) {
            return "editing"
        }
        if [kAXTextAreaRole, kAXStaticTextRole, "AXWebArea"].contains(role) { readable = true }
        if role == kAXWindowRole { return readable ? "reading" : "unknown" }
        current = elementAttribute(node, kAXParentAttribute)
    }
    return readable ? "reading" : "unknown"
}

/// Only the foreground app's focused element is inspected. No screen capture,
/// clipboard polling, password fields, or traversal of unrelated windows occurs.
private func focused() throws -> (NSRunningApplication, AXUIElement) {
    guard AXIsProcessTrusted() else { throw NativeError.message("请先在系统设置中为 Nobody 开启辅助功能权限。") }
    guard let app = NSWorkspace.shared.frontmostApplication, app.processIdentifier != getpid() else {
        throw NativeError.message("请先回到要翻译的应用。")
    }
    let root = AXUIElementCreateApplication(app.processIdentifier)
    AXUIElementSetMessagingTimeout(root, 0.25)
    // Chromium/Electron may lazily create its accessibility tree. Requesting its
    // AX activation attribute is limited to the app the user is interacting with.
    AXUIElementSetAttributeValue(root, "AXManualAccessibility" as CFString, kCFBooleanTrue)
    guard let raw = attribute(root, kAXFocusedUIElementAttribute), CFGetTypeID(raw) == AXUIElementGetTypeID() else {
        throw NativeError.message("这个应用没有提供可读取的文本焦点。可将文字粘贴到 Nobody 翻译。")
    }
    let element = unsafeBitCast(raw, to: AXUIElement.self)
    AXUIElementSetMessagingTimeout(element, 0.25)
    guard textAttribute(element, kAXSubroleAttribute) != kAXSecureTextFieldSubrole,
          (attribute(element, "AXProtectedContent") as? Bool) != true else {
        throw NativeError.message("密码和受保护输入框不参与翻译。")
    }
    return (app, element)
}

enum NativeError: Error { case message(String) }

/// Layout separators supplied by AX are not restricted to Unix LF. CRLF is a
/// single Swift Character; recognize it without splitting Unicode graphemes.
private func selectionLineBreak(_ character: Character) -> Bool {
    character.unicodeScalars.allSatisfy { [10, 13, 0x85, 0x2028, 0x2029].contains($0.value) }
}

/// Insert only proven boundaries, preserving every original byte and existing
/// separator. UTF-16 offsets from AX must land between complete graphemes, never
/// inside an emoji/surrogate pair. Adjacent existing breaks already carry layout.
private func insertingSelectionBreaks(_ text: String, _ breaks: [Int: Int]) -> String? {
    let boundaries = Dictionary(uniqueKeysWithValues: text.indices.map { ($0.utf16Offset(in: text), $0) })
    var output = ""
    var cursor = text.startIndex
    for offset in breaks.keys.sorted() {
        guard offset >= 0, offset <= text.utf16.count else { return nil }
        if offset == 0 || offset == text.utf16.count { continue }
        guard let index = boundaries[offset], let count = breaks[offset], (1...2).contains(count) else { return nil }
        let before = text[..<index].reversed().prefix(while: { $0.isWhitespace })
        let after = text[index...].prefix(while: { $0.isWhitespace })
        if before.contains(where: selectionLineBreak) || after.contains(where: selectionLineBreak) { continue }
        output += text[cursor..<index]
        output += String(repeating: "\n", count: count)
        cursor = index
    }
    output += text[cursor...]
    return output
}

/// A richer AX string may add layout but must describe exactly the same selected
/// characters. Never accept missing words, changed spaces/indentation, or text
/// from another range. Rebuild on the original rather than adopting provider text.
private func mergingSelectionLayout(_ original: String, _ candidate: String) -> String? {
    // Compare grapheme arrays before joining: joining first could conceal a
    // provider inserting a newline inside a combined accent or joined emoji.
    guard candidate.utf16.count <= 32000,
          Array(original).filter({ !selectionLineBreak($0) }) == Array(candidate).filter({ !selectionLineBreak($0) }) else { return nil }
    let offsets = original.indices.filter { !selectionLineBreak(original[$0]) }.map { $0.utf16Offset(in: original) }
    var position = 0
    var breaks: [Int: Int] = [:]
    for character in candidate {
        if selectionLineBreak(character) {
            if position > 0 && position < offsets.count {
                let extra = character == "\u{2029}" ? 2 : 1
                breaks[offsets[position]] = min(2, (breaks[offsets[position]] ?? 0) + extra)
            }
        } else { position += 1 }
    }
    return insertingSelectionBreaks(original, breaks)
}

/// Recover paragraph boundaries once, after a selection has stabilized. This is
/// deliberately absent from the 350 ms observer and never reads AXValue, another
/// window, the clipboard, or visual line ranges (soft wrapping is not a paragraph).
/// WebKit/Chromium text-marker extensions are optional capabilities: failed or
/// inconsistent APIs leave the original selection intact.
private func selectionLayout(_ request: [String: Any]) throws -> [String: Any] {
    guard let expected = request["expected"] as? [String: Any],
          let original = expected["text"] as? String, original.utf16.count <= 16000 else {
        throw NativeError.message("选区已失效，请重新划选。")
    }
    let (app, element) = try focused()
    let window = elementAttribute(element, kAXWindowAttribute)
    let context = "\(app.processIdentifier):\(window.map { CFHash($0) } ?? 0):\(CFHash(element))"
    let range = rangeAttribute(element)
    let selectionId = range.map { "\(CFHash(element)):\($0.location):\($0.length)" }
    guard expected["pid"] as? Int32 == app.processIdentifier,
          expected["contextId"] as? String == context,
          expected["selectionId"] as? String == selectionId,
          textAttribute(element, kAXSelectedTextAttribute) == original else {
        throw NativeError.message("选区已变化，请重新划选。")
    }
    var names: CFArray?
    AXUIElementCopyParameterizedAttributeNames(element, &names)
    let supported = Set(names as? [String] ?? [])
    let start = ProcessInfo.processInfo.systemUptime
    var queries = 0
    var layoutIssues = Set<String>()
    // An unresponsive accessibility provider must not delay typing or cause
    // unbounded main-thread work. The deadline is checked before every AX call.
    AXUIElementSetMessagingTimeout(element, 0.04)
    defer { AXUIElementSetMessagingTimeout(element, 0.25) }
    func query(_ name: String, _ value: CFTypeRef) -> CFTypeRef? {
        guard supported.contains(name) else { layoutIssues.insert("\(name):unsupported"); return nil }
        guard queries < 200, ProcessInfo.processInfo.systemUptime - start < 0.20 else {
            layoutIssues.insert("budget"); return nil
        }
        queries += 1
        let result = parameterizedAttribute(element, name, value)
        if result == nil { layoutIssues.insert("\(name):unavailable") }
        return result
    }
    var enriched = original
    var method = "plain"
    func consider(_ value: CFTypeRef?, _ source: String) {
        let candidate = (value as? NSAttributedString)?.string ?? (value as? String)
        if let candidate, let merged = mergingSelectionLayout(original, candidate),
           merged.filter({ selectionLineBreak($0) }).count > enriched.filter({ selectionLineBreak($0) }).count {
            enriched = merged
            method = source
        }
    }
    if var selectedRange = range, selectedRange.location >= 0, selectedRange.length > 0,
       selectedRange.length <= 16000, let value = AXValueCreate(.cfRange, &selectedRange) {
        consider(query("AXAttributedStringForRange", value), "attributed-range")
        consider(query("AXStringForRange", value), "text-range")
    }
    let markers = attribute(element, "AXSelectedTextMarkerRange")
    if let markers {
        consider(query("AXAttributedStringForTextMarkerRange", markers), "attributed-markers")
        consider(query("AXStringForTextMarkerRange", markers), "text-markers")
    }

    // On some web views even attributed text concatenates adjacent <p> nodes.
    // Use the actual selection's opaque endpoints: AXSelectedTextRange offsets
    // can be node-local while AXTextMarkerForIndex expects a document offset.
    // Normalizing endpoint order also covers selections dragged backwards.
    // Apple's marker functions are available on every supported macOS (13+).
    if let markers, CFGetTypeID(markers) == AXTextMarkerRangeGetTypeID(),
       case let markerRange = unsafeBitCast(markers, to: AXTextMarkerRange.self),
       let selected = query("AXTextMarkerRangeForUnorderedTextMarkers",
            [AXTextMarkerRangeCopyStartMarker(markerRange), AXTextMarkerRangeCopyEndMarker(markerRange)] as CFArray),
       CFGetTypeID(selected) == AXTextMarkerRangeGetTypeID(),
       query("AXStringForTextMarkerRange", selected) as? String == original {
        let first = AXTextMarkerRangeCopyStartMarker(unsafeBitCast(selected, to: AXTextMarkerRange.self))
        var marker: CFTypeRef = first
        var previousLength = 0
        var breaks: [Int: Int] = [:]
        var complete = false
        for _ in 0..<64 {
            guard let next = query("AXNextParagraphEndTextMarkerForTextMarker", marker), !CFEqual(marker, next),
                  let prefix = query("AXTextMarkerRangeForUnorderedTextMarkers", [first, next] as CFArray),
                  let length = (query("AXLengthForTextMarkerRange", prefix) as? NSNumber)?.intValue,
                  length > previousLength else { break }
            // A final paragraph may extend beyond a partial selection. Ask for
            // length first and stop at our boundary without reading its text.
            if length >= original.utf16.count { complete = true; break }
            guard let text = query("AXStringForTextMarkerRange", prefix) as? String,
                  text.utf16.count == length, original.hasPrefix(text) else { break }
            breaks[length] = 2
            previousLength = length
            marker = next
        }
        if complete, let recovered = insertingSelectionBreaks(original, breaks), recovered != original {
            // Merge the independent, validated layout sources without deleting
            // existing line breaks or replacing any of the selected characters.
            enriched = mergingSelectionLayout(enriched, recovered) ?? enriched
            method = "paragraph-markers"
        }
    }
    let currentRange = rangeAttribute(element)
    // Repeated identical text at another offset/control is still a new
    // selection. Validate focus as well as characters before returning layout.
    let (currentApp, currentElement) = try focused()
    guard currentApp.processIdentifier == app.processIdentifier, CFEqual(currentElement, element),
          textAttribute(element, kAXSelectedTextAttribute) == original,
          currentRange?.location == range?.location, currentRange?.length == range?.length else {
        throw NativeError.message("选区已变化，请重新划选。")
    }
    // Opt-in system debug logging records capability names/counts only. Never
    // log selected text, app titles, document contents, or persistent copies.
    let diagnostic = "method=\(method) range=\(range != nil) markers=\(markers != nil) sourceBreaks=\(original.filter { selectionLineBreak($0) }.count) resultBreaks=\(enriched.filter { selectionLineBreak($0) }.count) queries=\(queries) issues=\(layoutIssues.sorted().joined(separator: ","))"
    Logger(subsystem: "app.translateme.desktop", category: "selection-layout").debug("\(diagnostic, privacy: .public)")
    return ["text": enriched, "layoutSource": method]
}

/// Read only the selected range's rectangle. Some Chromium controls expose
/// text-marker ranges instead of UTF-16 ranges; neither path reads the window's
/// other text. A missing geometry API falls back to the current pointer.
private func selectedBounds(_ element: AXUIElement) -> CGRect? {
    var raw: CFTypeRef?
    if var range = rangeAttribute(element), range.length > 0,
       let value = AXValueCreate(.cfRange, &range) {
        AXUIElementCopyParameterizedAttributeValue(element, kAXBoundsForRangeParameterizedAttribute as CFString, value, &raw)
    }
    if raw == nil, let marker = attribute(element, "AXSelectedTextMarkerRange") {
        AXUIElementCopyParameterizedAttributeValue(element, "AXBoundsForTextMarkerRange" as CFString, marker, &raw)
    }
    guard let raw, CFGetTypeID(raw) == AXValueGetTypeID() else { return nil }
    var rect = CGRect.zero
    guard AXValueGetValue(unsafeBitCast(raw, to: AXValue.self), .cgRect, &rect),
          [rect.minX, rect.minY, rect.width, rect.height].allSatisfy({ $0.isFinite }),
          rect.width > 0, rect.height > 0 else { return nil }
    return rect
}

/// AX uses top-left screen points, AppKit uses bottom-left points, and Tao
/// flips its logical coordinates against CGDisplayPixelsHigh. Keep this
/// conversion at the native boundary so mixed-DPI screens are not double-scaled.
private func selectionAnchor(_ element: AXUIElement) -> [String: Any]? {
    let screens = NSScreen.screens
    guard let primary = screens.first else { return nil }
    let primaryTop = primary.frame.maxY
    let taoTop = CGFloat(CGDisplayPixelsHigh(CGMainDisplayID()))
    let bounds = selectedBounds(element)
    let pointer = NSEvent.mouseLocation
    let raw = bounds ?? CGRect(x: pointer.x, y: primaryTop - pointer.y, width: 1, height: 1)
    let center = NSPoint(x: raw.midX, y: primaryTop - raw.midY)
    let screen = screens.first(where: { $0.frame.contains(center) })
        ?? screens.max(by: {
            let a = CGRect(x: $0.frame.minX, y: primaryTop - $0.frame.maxY, width: $0.frame.width, height: $0.frame.height)
            let b = CGRect(x: $1.frame.minX, y: primaryTop - $1.frame.maxY, width: $1.frame.width, height: $1.frame.height)
            return a.intersection(raw).width * a.intersection(raw).height < b.intersection(raw).width * b.intersection(raw).height
        }) ?? primary
    let work = CGRect(x: screen.visibleFrame.minX, y: primaryTop - screen.visibleFrame.maxY,
                      width: screen.visibleFrame.width, height: screen.visibleFrame.height)
    var visible = raw.intersection(work)
    // Chromium may expose a selected response through a focused container with
    // unrelated bounds. Clip only to its owning window, not that container.
    // A previously valid range leaving the window can still hide the popover.
    for owner in [attribute(element, kAXWindowAttribute).flatMap {
        CFGetTypeID($0) == AXUIElementGetTypeID() ? unsafeBitCast($0, to: AXUIElement.self) : nil
    }].compactMap({ $0 }) {
        if let pos = attribute(owner, kAXPositionAttribute), let size = attribute(owner, kAXSizeAttribute),
           CFGetTypeID(pos) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() {
            var point = CGPoint.zero
            var extent = CGSize.zero
            if AXValueGetValue(unsafeBitCast(pos, to: AXValue.self), .cgPoint, &point),
               AXValueGetValue(unsafeBitCast(size, to: AXValue.self), .cgSize, &extent),
               extent.width > 0, extent.height > 0, bounds != nil {
                visible = visible.intersection(CGRect(origin: point, size: extent))
            }
        }
    }
    func json(_ rect: CGRect) -> [String: CGFloat] {
        ["x": rect.minX, "y": rect.minY + taoTop - primaryTop, "width": rect.width, "height": rect.height]
    }
    let isVisible = !visible.isNull && !visible.isEmpty
    // Off-screen APIs may return a zero rectangle. Preserve visible=false with
    // finite placeholder dimensions so the shared layer does not mistake this
    // for an unsupported API and fall back to a visible pointer anchor.
    let hidden = CGRect(x: raw.minX, y: raw.minY, width: max(1, raw.width), height: max(1, raw.height))
    return ["rect": json(isVisible ? visible : hidden), "workArea": json(work), "scale": 1,
            "space": "logical", "kind": bounds == nil ? "cursor" : "selection", "visible": isVisible]
}

/// The WebView's CSS cannot color the native titlebar. Use the header's
/// composited light/dark palette for the window backing, with AppKit resolving
/// a dynamic color on appearance changes. Keep the native title/buttons and
/// content geometry; no full-size overlay, polling or theme IPC is needed.
private func configureMainWindowAppearance() {
    guard let window = NSApp.windows.first(where: { $0.title == "Nobody" }) else { return }
    // Tauri's default Visible style still sets fullSizeContentView. Remove it
    // before making the bar transparent, or the WebView moves behind the
    // traffic lights and the logo overlaps them (notably on recent macOS).
    let frame = window.frame
    window.styleMask.remove(.fullSizeContentView)
    window.titlebarAppearsTransparent = true
    // Changing this mask can grow the outer frame by the titlebar height.
    // Keep the existing window size/position and let its content resize inside.
    window.setFrame(frame, display: false)
    window.titlebarSeparatorStyle = .none
    window.backgroundColor = NSColor(name: "NobodyWindowBackground") { appearance in
        // Match Aqua/Dark Aqua rather than reading a cached system preference,
        // so automatic theme changes and increased-contrast variants still work.
        let dark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        return dark
            ? NSColor(srgbRed: 28.0 / 255, green: 39.0 / 255, blue: 31.0 / 255, alpha: 1)
            : NSColor(srgbRed: 248.0 / 255, green: 250.0 / 255, blue: 247.0 / 255, alpha: 1)
    }
}

/// Configure only our own result window. Floating above ordinary windows is
/// insufficient in another app's full-screen Space or Stage Manager set;
/// these public collection behaviors let the popover accompany its source.
private func configurePopover() throws {
    guard let window = NSApp.windows.first(where: { $0.title == "Nobody · 译文" }) else {
        throw NativeError.message("译文浮窗尚未创建。")
    }
    var behavior = window.collectionBehavior
    behavior.remove([.fullScreenPrimary, .fullScreenNone])
    behavior.formUnion([.canJoinAllSpaces, .fullScreenAuxiliary])
    if #available(macOS 13.0, *) {
        behavior.remove([.primary, .auxiliary])
        behavior.insert(.canJoinAllApplications)
    }
    window.collectionBehavior = behavior
    window.hidesOnDeactivate = false
}

/// A selection peek never creates a writable ticket. Explicit write shortcuts
/// capture identity, original value and UTF-16 range for compare-before-replace.
private func capture(_ request: [String: Any]) throws -> [String: Any] {
    let automatic = request["op"] as? String == "selection"
    let allowed = request["allowedApps"] as? [Any] ?? []
    guard let foreground = NSWorkspace.shared.frontmostApplication else { return ["ignored": true] }
    if automatic { selectionMouse.prepareAccessibility(foreground) }
    var metadata = selectionMouse.metadata(foreground.processIdentifier)
    metadata["mouseDown"] = NSEvent.pressedMouseButtons & 1 != 0
    if !needsSelectionCapture(request, metadata, foreground.processIdentifier) {
        // No focused-element, ancestry, text or geometry IPC occurs at idle.
        metadata.merge(["ignored": true, "reason": "idle", "pid": foreground.processIdentifier]) { _, next in next }
        return metadata
    }
    let appAllowed = applicationAllowed(foreground, allowed)
    let tracking = request["tracking"] as? [String: Any] ?? [:]
    // Following an explicitly requested result is allowed only for its exact
    // source PID and control. New captures in other applications stop here.
    if automatic && !appAllowed && tracking["pid"] as? Int32 != foreground.processIdentifier {
        return ["ignored": true, "reason": "application", "pid": foreground.processIdentifier]
    }
    let (app, element) = try focused()
    guard app.processIdentifier == foreground.processIdentifier else { return ["ignored": true] }
    let window = elementAttribute(element, kAXWindowAttribute)
    let contextId = "\(app.processIdentifier):\(window.map { CFHash($0) } ?? 0):\(CFHash(element))"
    let context = automatic ? automaticContext(element) : "manual"
    var result: [String: Any] = ["app": app.localizedName ?? "App", "pid": app.processIdentifier,
        "contextId": contextId, "autoEligible": appAllowed && context == "reading",
        "mouseDown": NSEvent.pressedMouseButtons & 1 != 0]
    if automatic {
        result.merge(metadata) { _, next in next }
        let following = tracking["pid"] as? Int32 == app.processIdentifier && tracking["contextId"] as? String == contextId
        if (!appAllowed || context != "reading") && !following {
            result["ignored"] = true
            result["reason"] = context
            return result
        }
    }
    let selected = textAttribute(element, kAXSelectedTextAttribute) ?? ""
    // AXValue can be an entire chat/document. Only whole-input translation or
    // a compare-before-replace ticket needs it; reading a selection never does.
    let value = request["whole"] as? Bool == true || request["ticket"] as? Bool == true
        ? textAttribute(element, kAXValueAttribute) : nil
    let role = textAttribute(element, kAXRoleAttribute) ?? ""
    let editable = [kAXTextAreaRole, kAXTextFieldRole, kAXComboBoxRole].contains(role)
        && (isSettable(element, kAXValueAttribute) || isSettable(element, kAXSelectedTextAttribute) || isSettable(element, kAXSelectedTextRangeAttribute))
    let whole = selected.isEmpty && (request["whole"] as? Bool == true) && editable
    let text = whole ? (value ?? "") : selected
    if automatic && text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return result }
    guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw NativeError.message("没有选中文字；写入翻译需要将光标放在可编辑的输入框。") }
    guard text.utf16.count <= 16000 else { throw NativeError.message("本次文字超过 16,000 字符，请分段翻译。") }
    result.merge(["text": text, "editable": editable, "whole": whole]) { _, next in next }
    if let range = rangeAttribute(element) {
        result["selectionId"] = "\(CFHash(element)):\(range.location):\(range.length)"
    }
    if request["geometry"] as? Bool == true { result["anchor"] = selectionAnchor(element) }
    if request["ticket"] as? Bool == true {
        snapshots = snapshots.filter { Date().timeIntervalSince($0.value.created) < 120 }
        if snapshots.count >= 16 { snapshots.removeAll() }
        let id = UUID().uuidString
        snapshots[id] = Snapshot(element: element, pid: app.processIdentifier, value: value, selected: selected, range: rangeAttribute(element), whole: whole, created: Date())
        // Failed/cancelled engine calls may never consume their ticket. Release
        // its original document and AX objects even if no later capture occurs.
        DispatchQueue.main.asyncAfter(deadline: .now() + 120) { snapshots.removeValue(forKey: id) }
        result["ticket"] = id
    }
    return result
}

private func matches(_ snapshot: Snapshot) -> Bool {
    guard Date().timeIntervalSince(snapshot.created) < 120,
          let (app, element) = try? focused(), app.processIdentifier == snapshot.pid,
          CFEqual(element, snapshot.element),
          textAttribute(element, kAXValueAttribute) == snapshot.value,
          (textAttribute(element, kAXSelectedTextAttribute) ?? "") == snapshot.selected else { return false }
    let range = rangeAttribute(element)
    return range?.location == snapshot.range?.location && range?.length == snapshot.range?.length
}

/// Use the target editor's ordinary paste path for Undo support. Restore every
/// original pasteboard flavor only if the user has not copied something newer.
private func replace(_ request: [String: Any], _ reply: @escaping Reply, _ context: UnsafeMutableRawPointer?) throws {
    guard let id = request["ticket"] as? String, let snapshot = snapshots.removeValue(forKey: id),
          let translation = request["text"] as? String, !translation.isEmpty else { throw NativeError.message("原输入已过期，请重新按翻译快捷键。") }
    guard matches(snapshot) else { throw NativeError.message("输入框、选区或原文已变化；译文已保留，请手动复制。") }
    if snapshot.whole {
        var range = CFRange(location: 0, length: (snapshot.value ?? "").utf16.count)
        guard let axRange = AXValueCreate(.cfRange, &range),
              AXUIElementSetAttributeValue(snapshot.element, kAXSelectedTextRangeAttribute as CFString, axRange) == .success else {
            throw NativeError.message("这个输入框不支持安全全选；请先选中文字，再按快捷键。")
        }
    }
    // Construct events before modifying clipboard state, so allocation failure
    // cannot leave a temporary translation on the user's pasteboard.
    guard let down = CGEvent(keyboardEventSource: nil, virtualKey: 9, keyDown: true),
          let up = CGEvent(keyboardEventSource: nil, virtualKey: 9, keyDown: false) else {
        throw NativeError.message("无法创建粘贴事件。")
    }
    let board = NSPasteboard.general
    let saved: [[NSPasteboard.PasteboardType: Data]] = (board.pasteboardItems ?? []).map { item in
        var record: [NSPasteboard.PasteboardType: Data] = [:]
        for type in item.types { if let data = item.data(forType: type) { record[type] = data } }
        return record
    }
    board.clearContents()
    board.setString(translation, forType: .string)
    let temporaryVersion = board.changeCount
    down.flags = .maskCommand; up.flags = .maskCommand
    down.post(tap: .cghidEventTap); up.post(tap: .cghidEventTap)
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.65) {
        if board.changeCount == temporaryVersion {
            board.clearContents()
            let items = saved.map { record -> NSPasteboardItem in
                let item = NSPasteboardItem()
                for (type, data) in record { item.setData(data, forType: type) }
                return item
            }
            if !items.isEmpty { board.writeObjects(items) }
        }
        let updated = textAttribute(snapshot.element, kAXValueAttribute)
        var expected: String?
        if snapshot.whole { expected = translation }
        else if let original = snapshot.value, let r = snapshot.range,
                r.location >= 0, r.length >= 0, r.location + r.length <= original.utf16.count {
            expected = (original as NSString).replacingCharacters(in: NSRange(location: r.location, length: r.length), with: translation)
        }
        let confirmed = expected != nil && updated == expected
        respond(["ok": true, "confirmed": confirmed], reply, context)
    }
}

#if canImport(Translation)
/// Match the language objects advertised by Translation, instead of mixing NL's
/// bare identifiers with regional defaults. Preserve script distinctions (notably
/// Hans/Hant); source/target reversal must resolve to the same installed assets.
private func canonicalTranslationLanguage(_ language: Locale.Language, supported: [Locale.Language]) -> Locale.Language? {
    let matches = supported.filter { $0.languageCode == language.languageCode && $0.script == language.script }
    // The supported list can put British English first. Prefer the requested
    // (including inferred) region, or bare "en" would switch an installed US
    // model to a missing UK model and create exactly the redundant download.
    return matches.first { $0.maximalIdentifier == language.maximalIdentifier } ?? matches.first
}

/// A language detector always has a top guess, even for "Error" or "git status".
/// Confidence plus runner-up separation is required before that guess can cause
/// a download. These conservative thresholds gate downloads only: installed
/// languages remain usable for short selections without additional prompts.
private func canRequestTranslationDownload(_ hypotheses: [NLLanguage: Double]) -> Bool {
    let scores = hypotheses.values.sorted(by: >)
    guard let first = scores.first else { return false }
    return first >= 0.80 && first - (scores.dropFirst().first ?? 0) >= 0.20
}

/// Re-query the OS for each request, including after relaunch. The reverse pair
/// is evidence that the same two language assets are installed; direction is
/// not a second download. Never persist a "downloaded" flag that could become
/// stale after macOS/user removal. An unsupported forward pair stays unsupported.
@available(macOS 15.0, *)
private func translationPairStatus(source: Locale.Language, target: Locale.Language,
    lookup: (Locale.Language, Locale.Language) async -> LanguageAvailability.Status) async -> LanguageAvailability.Status {
    let forward = await lookup(source, target)
    guard forward == .supported else { return forward }
    let reverse = await lookup(target, source)
    return reverse == .installed ? .installed : forward
}

/// Both UI-backed and installed-language sessions use the same ordering. The
/// framework may return batch responses out of order; IDs refer to input slots.
@available(macOS 15.0, *)
private func translateBatch(_ texts: [String], using session: TranslationSession) async throws -> [String] {
    let responses = try await session.translations(from: texts.enumerated().map {
        TranslationSession.Request(sourceText: $0.element, clientIdentifier: String($0.offset))
    })
    var ordered = Array(repeating: "", count: texts.count)
    for response in responses {
        guard let id = response.clientIdentifier.flatMap(Int.init), ordered.indices.contains(id) else { continue }
        ordered[id] = response.targetText
    }
    return ordered
}

/// A visible host is needed for Apple's download consent and for the legacy
/// session API. An ordinary translation must not be described as a download.
@available(macOS 15.0, *)
private struct TranslationHost: View {
    let configuration: TranslationSession.Configuration
    let texts: [String]
    let needsPreparation: Bool
    let begin: () -> Bool
    let completion: (Result<[String], Error>) -> Void
    var body: some View {
        VStack(spacing: 12) {
            ProgressView()
            Text(needsPreparation ? "正在准备所需语言" : "正在翻译…").font(.headline)
            Text(needsPreparation ? "首次使用这组语言，请在系统提示中确认下载。双向翻译共用语言包。" : "正在使用已安装的语言包。")
                .font(.caption).foregroundStyle(.secondary)
        }.padding(24).frame(width: 320, height: 130)
        .translationTask(configuration) { session in
            // SwiftUI can restart a task when its host changes. One native
            // request owns one translation/download flow and one FFI callback.
            guard begin() else { return }
            do {
                // Translation itself requests missing assets and waits for the
                // download. A separate prepareTranslation followed by translate
                // creates two entry points into Apple's consent/download UI.
                completion(.success(try await translateBatch(texts, using: session)))
            } catch { completion(.failure(error)) }
        }
    }
}
private var translationPanel: NSPanel?
// Main-queue state covers both visible and headless requests. A nil panel no
// longer implies idle when installed-language translation runs without UI.
private var translationInProgress = false

/// Resolve a stable language pair and its installed state before showing any
/// download UI. macOS 26+ installed sessions cannot request downloads; failures
/// are reported without retrying through a second, consent-capable session.
@available(macOS 15.0, *)
private func translate(_ request: [String: Any], _ reply: @escaping Reply, _ context: UnsafeMutableRawPointer?) {
    guard !translationInProgress else { respond(["error": "系统翻译正在运行，请稍后再试。"], reply, context); return }
    let texts = request["texts"] as? [String] ?? []
    let code = request["target"] as? String ?? "en"
    let recognizer = NLLanguageRecognizer()
    recognizer.processString(texts.joined(separator: "\n"))
    let hypotheses = recognizer.languageHypotheses(withMaximum: 2)
    guard let detected = recognizer.dominantLanguage else {
        respond(["error": "无法确定选中文字的语言，请选择完整句子或切换 LLM；未请求下载语言包。"], reply, context)
        return
    }
    translationInProgress = true
    var finished = false
    let finish: (Result<[String], Error>) -> Void = { result in
        DispatchQueue.main.async {
            // Duplicate completion would free Rust's callback context twice.
            // Protect both the download lifecycle and the existing FFI contract.
            guard !finished else { return }
            finished = true
            translationPanel?.orderOut(nil)
            translationPanel = nil
            translationInProgress = false
            switch result {
            case .success(let translated): respond(["texts": translated], reply, context)
            case .failure(let error):
                let detail = (error as? LocalizedError)?.failureReason ?? error.localizedDescription
                // Network, unsupported language and cancellation errors are not
                // all download failures; preserve the actual framework reason.
                respond(["error": "系统翻译失败：\(detail)"], reply, context)
            }
        }
    }
    Task { @MainActor in
        let availability = LanguageAvailability()
        let supported = await availability.supportedLanguages
        guard let source = canonicalTranslationLanguage(Locale.Language(identifier: detected.rawValue), supported: supported),
              let target = canonicalTranslationLanguage(Locale.Language(identifier: code), supported: supported) else {
            finish(.failure(NSError(domain: "Nobody.Translation", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "系统不支持当前语言，请选择完整句子或切换 LLM。"])))
            return
        }
        if source.languageCode == target.languageCode {
            // Simplified and Traditional Chinese share a language code. Do not
            // silently claim conversion when Apple disallows same-language pairs.
            if source.script != target.script {
                finish(.failure(NSError(domain: "Nobody.Translation", code: 2,
                    userInfo: [NSLocalizedDescriptionKey: "系统翻译不支持同一语言的书写系统转换，请切换 LLM。"])))
            } else { finish(.success(texts)) }
            return
        }
        let status = await translationPairStatus(source: source, target: target) {
            await availability.status(from: $0, to: $1)
        }
        guard status != .unsupported else {
            finish(.failure(NSError(domain: "Nobody.Translation", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "系统不支持当前语言组合，请切换 LLM。"])))
            return
        }
        let needsPreparation = status != .installed
        // Availability can lag behind a completed download. Probe a session
        // that cannot request downloads before deciding to open a consent host.
        // If the OS claims installed but the assets are unavailable, report the
        // actual error rather than automatically starting another download flow.
        #if compiler(>=6.2)
        if #available(macOS 26.0, *) {
            let session = TranslationSession(installedSource: source, target: target)
            let ready = await session.isReady
            Logger(subsystem: "app.translateme.desktop", category: "translation-models").debug("source=\(source.minimalIdentifier, privacy: .public) target=\(target.minimalIdentifier, privacy: .public) installed=\(!needsPreparation) ready=\(ready) canDownload=false")
            if !needsPreparation || ready {
                do { finish(.success(try await translateBatch(texts, using: session))) }
                catch { finish(.failure(error)) }
                return
            }
        }
        #endif
        guard !needsPreparation || canRequestTranslationDownload(hypotheses) else {
            finish(.failure(NSError(domain: "Nobody.Translation", code: 3,
                userInfo: [NSLocalizedDescriptionKey: "选中文字的语种不明确，已避免下载可能用不到的语言包。请选择完整句子或切换 LLM。"])))
            return
        }
        // Debug metadata only: no original/translated text or persistent history.
        Logger(subsystem: "app.translateme.desktop", category: "translation-models").debug("source=\(source.minimalIdentifier, privacy: .public) target=\(target.minimalIdentifier, privacy: .public) installed=\(!needsPreparation)")
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 368, height: 178), styleMask: [.titled, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.title = "Nobody · Apple 翻译"
        panel.level = .floating
        panel.isReleasedWhenClosed = false
        // Keep the one-shot flag in the native request, not the SwiftUI view:
        // rebuilding a host must not reset it and restart an in-flight download.
        var hostStarted = false
        panel.contentView = NSHostingView(rootView: TranslationHost(configuration: .init(source: source, target: target),
            texts: texts, needsPreparation: needsPreparation, begin: {
                guard !hostStarted else { return false }
                hostStarted = true
                return true
            }, completion: finish))
        panel.center()
        translationPanel = panel
        panel.orderFrontRegardless()
    }
}
#endif

/// A single exported ABI keeps platform details out of Rust. Requests are JSON
/// values (never shell commands) and all results, including failures, reply once.
@_cdecl("tm_native_request")
public func nativeRequest(_ json: UnsafePointer<CChar>, _ reply: @escaping @convention(c) (UnsafePointer<CChar>?, UnsafeMutableRawPointer?) -> Void, _ context: UnsafeMutableRawPointer?) {
    let data = Data(String(cString: json).utf8)
    DispatchQueue.main.async {
        do {
            guard let request = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw NativeError.message("无效的原生请求。") }
            if request["op"] as? String == "selection" {
                selectionMouse.configure(request["allowedApps"] as? [Any] ?? [])
            }
            switch request["op"] as? String {
            case "background":
                scheduleBackgroundCleanup()
                respond(["ok": true], reply, context)
            case "pickApplications": pickApplications(reply, context)
            case "applicationIcons":
                respond(["icons": applicationIcons(request["apps"] as? [Any] ?? [])], reply, context)
            case "validateApplications":
                try validateApplications(request["apps"] as? [Any] ?? [])
                respond(["ok": true], reply, context)
            case "configurePopover":
                // Both windows already exist at this one-time startup call.
                // The main titlebar is styled independently of the popover.
                configureMainWindowAppearance()
                try configurePopover()
                respond(["ok": true], reply, context)
            case "status":
                var translation = false
                #if canImport(Translation)
                if #available(macOS 15.0, *) { translation = true }
                #endif
                respond(["accessibility": AXIsProcessTrusted(), "systemTranslation": translation, "platform": "macOS", "defaultApps": defaultApplications()], reply, context)
            case "permission":
                let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
                let trusted = AXIsProcessTrustedWithOptions(options)
                if !trusted, let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility") { NSWorkspace.shared.open(url) }
                respond(["accessibility": trusted], reply, context)
            case "selection" where NSWorkspace.shared.frontmostApplication?.processIdentifier == getpid():
                // Clicking or dragging our own popover must not dismiss it as a
                // lost source selection. No other application's text is read.
                respond(["self": true], reply, context)
            case "capture", "selection": respond(try capture(request), reply, context)
            case "selectionLayout": respond(try selectionLayout(request), reply, context)
            case "replace": try replace(request, reply, context)
            case "copy":
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(request["text"] as? String ?? "", forType: .string)
                respond(["ok": true], reply, context)
            case "translate":
                #if canImport(Translation)
                if #available(macOS 15.0, *) { translate(request, reply, context); return }
                #endif
                throw NativeError.message("此构建未包含 Apple Translation。请用 macOS 15+ SDK 重新构建，或在设置中选择 LLM。")
            default: throw NativeError.message("未知的原生命令。")
            }
        } catch NativeError.message(let message) { respond(["error": message], reply, context) }
        catch { respond(["error": error.localizedDescription], reply, context) }
    }
}
