// Native macOS adapter. All AppKit / AX access is serialized on the main queue.
// Rust owns the request lifetime; callbacks copy the JSON before this stack ends.
import AppKit
import ApplicationServices
import Foundation
import NaturalLanguage
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

/// A selection peek never creates a writable ticket. Explicit write shortcuts
/// capture identity, original value and UTF-16 range for compare-before-replace.
private func capture(_ request: [String: Any]) throws -> [String: Any] {
    let (app, element) = try focused()
    let selected = textAttribute(element, kAXSelectedTextAttribute) ?? ""
    let value = textAttribute(element, kAXValueAttribute)
    let role = textAttribute(element, kAXRoleAttribute) ?? ""
    let editable = [kAXTextAreaRole, kAXTextFieldRole, kAXComboBoxRole].contains(role)
        && (isSettable(element, kAXValueAttribute) || isSettable(element, kAXSelectedTextAttribute) || isSettable(element, kAXSelectedTextRangeAttribute))
    let whole = selected.isEmpty && (request["whole"] as? Bool == true) && editable
    let text = whole ? (value ?? "") : selected
    guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw NativeError.message("没有选中文字；写入翻译需要将光标放在可编辑的输入框。") }
    guard text.utf16.count <= 16000 else { throw NativeError.message("本次文字超过 16,000 字符，请分段翻译。") }
    var result: [String: Any] = ["text": text, "app": app.localizedName ?? "App", "pid": app.processIdentifier, "editable": editable, "whole": whole]
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
@available(macOS 15.0, *)
private struct TranslationHost: View {
    let source: Locale.Language?
    let target: Locale.Language
    let texts: [String]
    let completion: (Result<[String], Error>) -> Void
    var body: some View {
        VStack(spacing: 12) {
            ProgressView()
            Text("正在准备系统翻译").font(.headline)
            Text("首次使用时，macOS 可能需要下载语言包。")
                .font(.caption).foregroundStyle(.secondary)
        }.padding(24).frame(width: 320, height: 130)
        .translationTask(source: source, target: target) { session in
            do {
                // Prepare uses Apple's own download consent UI; it is never
                // replaced by an implicit network fallback to an LLM provider.
                try await session.prepareTranslation()
                let responses = try await session.translations(from: texts.enumerated().map {
                    TranslationSession.Request(sourceText: $0.element, clientIdentifier: String($0.offset))
                })
                var ordered = Array(repeating: "", count: texts.count)
                for response in responses {
                    guard let id = response.clientIdentifier.flatMap(Int.init), ordered.indices.contains(id) else { continue }
                    ordered[id] = response.targetText
                }
                completion(.success(ordered))
            } catch { completion(.failure(error)) }
        }
    }
}
private var translationPanel: NSPanel?

@available(macOS 15.0, *)
private func translate(_ request: [String: Any], _ reply: @escaping Reply, _ context: UnsafeMutableRawPointer?) {
    guard translationPanel == nil else { respond(["error": "系统翻译正在运行，请稍后再试。"], reply, context); return }
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
    let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 368, height: 178), styleMask: [.titled, .nonactivatingPanel], backing: .buffered, defer: false)
    panel.title = "TranslateMe · Apple 翻译"
    panel.level = .floating
    panel.isReleasedWhenClosed = false
    let content = TranslationHost(source: source, target: target, texts: texts) { result in
        DispatchQueue.main.async {
            translationPanel?.orderOut(nil)
            translationPanel = nil
            switch result {
            case .success(let translated): respond(["texts": translated], reply, context)
            case .failure(let error):
                let detail = (error as? LocalizedError)?.failureReason ?? error.localizedDescription
                respond(["error": "系统翻译失败：\(detail)。首次使用请等待语言包下载完成后重试。"], reply, context)
            }
        }
    }
    panel.contentView = NSHostingView(rootView: content)
    panel.center()
    translationPanel = panel
    panel.orderFrontRegardless()
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
            switch request["op"] as? String {
            case "status":
                var translation = false
                #if canImport(Translation)
                if #available(macOS 15.0, *) { translation = true }
                #endif
                respond(["accessibility": AXIsProcessTrusted(), "systemTranslation": translation, "platform": "macOS"], reply, context)
            case "permission":
                let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
                let trusted = AXIsProcessTrustedWithOptions(options)
                if !trusted, let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility") { NSWorkspace.shared.open(url) }
                respond(["accessibility": trusted], reply, context)
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
