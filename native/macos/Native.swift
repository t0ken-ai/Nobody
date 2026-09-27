// Native macOS adapter. All AppKit / AX access is serialized on the main queue.
// Rust owns the request lifetime; callbacks copy the JSON before this stack ends.
import AppKit
import ApplicationServices
import Foundation
import NaturalLanguage
import UniformTypeIdentifiers
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

    func configure(_ applications: [Any]) {
        allowed = applications
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
    guard AXIsProcessTrusted() else { throw NativeError.message("请先在系统设置中为 TranslateMe 开启辅助功能权限。") }
    guard let app = NSWorkspace.shared.frontmostApplication, app.processIdentifier != getpid() else {
        throw NativeError.message("请先回到要翻译的应用。")
    }
    let root = AXUIElementCreateApplication(app.processIdentifier)
    AXUIElementSetMessagingTimeout(root, 0.25)
    // Chromium/Electron may lazily create its accessibility tree. Requesting its
    // AX activation attribute is limited to the app the user is interacting with.
    AXUIElementSetAttributeValue(root, "AXManualAccessibility" as CFString, kCFBooleanTrue)
    guard let raw = attribute(root, kAXFocusedUIElementAttribute), CFGetTypeID(raw) == AXUIElementGetTypeID() else {
        throw NativeError.message("这个应用没有提供可读取的文本焦点。可将文字粘贴到 TranslateMe 翻译。")
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

/// Configure only our own result window. Floating above ordinary windows is
/// insufficient in another app's full-screen Space or Stage Manager set;
/// these public collection behaviors let the popover accompany its source.
private func configurePopover() throws {
    guard let window = NSApp.windows.first(where: { $0.title == "TranslateMe · 译文" }) else {
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
        result.merge(selectionMouse.metadata(app.processIdentifier)) { _, next in next }
        let following = tracking["pid"] as? Int32 == app.processIdentifier && tracking["contextId"] as? String == contextId
        if (!appAllowed || context != "reading") && !following {
            result["ignored"] = true
            result["reason"] = context
            return result
        }
    }
    let selected = textAttribute(element, kAXSelectedTextAttribute) ?? ""
    let value = textAttribute(element, kAXValueAttribute)
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
    let source: Locale.Language?
    let target: Locale.Language
    let texts: [String]
    let needsPreparation: Bool
    let completion: (Result<[String], Error>) -> Void
    var body: some View {
        VStack(spacing: 12) {
            ProgressView()
            Text(needsPreparation ? "正在准备所需语言" : "正在翻译…").font(.headline)
            Text(needsPreparation ? "缺少对应语言包，请在系统提示中确认下载。" : "系统会在需要时提示确认语言。")
                .font(.caption).foregroundStyle(.secondary)
        }.padding(24).frame(width: 320, height: 130)
        .translationTask(source: source, target: target) { session in
            do {
                // Only request preparation for a known, missing language pair.
                // With a nil source, prepareTranslation cannot identify a
                // language; translating the real text lets Apple identify it.
                if needsPreparation {
                    try await session.prepareTranslation()
                }
                completion(.success(try await translateBatch(texts, using: session)))
            } catch { completion(.failure(error)) }
        }
    }
}
private var translationPanel: NSPanel?
// Main-queue state covers both visible and headless requests. A nil panel no
// longer implies idle when installed-language translation runs without UI.
private var translationInProgress = false

/// Query the current language installation state on every request: macOS may
/// remove downloaded models. macOS 26+ can translate installed pairs without a
/// SwiftUI panel; missing pairs retain Apple's standard download consent flow.
@available(macOS 15.0, *)
private func translate(_ request: [String: Any], _ reply: @escaping Reply, _ context: UnsafeMutableRawPointer?) {
    guard !translationInProgress else { respond(["error": "系统翻译正在运行，请稍后再试。"], reply, context); return }
    let texts = request["texts"] as? [String] ?? []
    let code = request["target"] as? String ?? "en"
    let recognizer = NLLanguageRecognizer()
    recognizer.processString(texts.joined(separator: "\n"))
    let source = recognizer.dominantLanguage.map { Locale.Language(identifier: $0.rawValue) }
    let target = Locale.Language(identifier: code)
    if source?.languageCode == target.languageCode {
        // Simplified and Traditional Chinese share a language code. Do not
        // silently claim conversion when Apple disallows same-language pairs.
        if source?.script != target.script {
            respond(["error": "系统翻译不支持同一语言的书写系统转换，请切换 LLM。"], reply, context)
        } else { respond(["texts": texts], reply, context) }
        return
    }
    translationInProgress = true
    let finish: (Result<[String], Error>) -> Void = { result in
        DispatchQueue.main.async {
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
        var needsPreparation = false
        if let source {
            let status = await LanguageAvailability().status(from: source, to: target)
            if status == .unsupported {
                finish(.failure(NSError(domain: "TranslateMe.Translation", code: 1,
                    userInfo: [NSLocalizedDescriptionKey: "系统不支持当前语言组合，请切换 LLM。"])))
                return
            }
            needsPreparation = status == .supported
            // Xcode 26 / Swift 6.2 introduced the headless initializer. Older
            // Apple toolchains keep compiling the macOS 15 UI-backed path.
            #if compiler(>=6.2)
            if #available(macOS 26.0, *), status == .installed {
                do {
                    let session = TranslationSession(installedSource: source, target: target)
                    finish(.success(try await translateBatch(texts, using: session)))
                } catch { finish(.failure(error)) }
                return
            }
            #endif
        }
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 368, height: 178), styleMask: [.titled, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.title = "TranslateMe · Apple 翻译"
        panel.level = .floating
        panel.isReleasedWhenClosed = false
        panel.contentView = NSHostingView(rootView: TranslationHost(source: source, target: target,
            texts: texts, needsPreparation: needsPreparation, completion: finish))
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
            case "pickApplications": pickApplications(reply, context)
            case "validateApplications":
                try validateApplications(request["apps"] as? [Any] ?? [])
                respond(["ok": true], reply, context)
            case "configurePopover":
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
