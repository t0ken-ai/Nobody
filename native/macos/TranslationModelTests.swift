// Appended to the production native adapter. Synthetic policies below exercise
// reversed pairs, stale state, and ambiguous selections without downloading any
// models, reading selections, or using credentials. --live additionally checks
// existing English/Chinese models through download-incapable sessions only.
#if canImport(Translation)
@available(macOS 15.0, *)
private func runTranslationModelTests() async throws {
    let en = Locale.Language(identifier: "en-US")
    let zh = Locale.Language(identifier: "zh-Hans-CN")
    let traditional = Locale.Language(identifier: "zh-Hant-TW")
    let supported = [en, zh, traditional]
    var checks = 0
    func check(_ condition: Bool, _ name: String) {
        precondition(condition, "Translation model regression: \(name)")
        checks += 1
    }
    check(canonicalTranslationLanguage(Locale.Language(identifier: "en"), supported: supported) == en,
        "bare English uses Apple's installed variant")
    check(canonicalTranslationLanguage(Locale.Language(identifier: "en-GB"), supported: supported) == en,
        "English regional variants do not create a new model identity")
    let british = Locale.Language(identifier: "en-GB")
    check(canonicalTranslationLanguage(Locale.Language(identifier: "en"), supported: [british, en, zh]) == en,
        "supported list order cannot switch bare English to a missing UK model")
    check(canonicalTranslationLanguage(british, supported: [en, british, zh]) == british,
        "explicit region is preferred when the framework supports it")
    check(canonicalTranslationLanguage(Locale.Language(identifier: "zh-CN"), supported: supported) == zh,
        "simplified Chinese aliases resolve identically")
    check(canonicalTranslationLanguage(Locale.Language(identifier: "zh-TW"), supported: supported) == traditional,
        "traditional Chinese stays distinct")
    check(canonicalTranslationLanguage(traditional, supported: [en, zh]) == nil,
        "never replace a missing script with the wrong Chinese model")
    check(canonicalTranslationLanguage(Locale.Language(identifier: "xx"), supported: supported) == nil,
        "unsupported language is not mapped to an unrelated installed language")

    // A fake OS changes from missing -> installed -> removed. A fresh request
    // must honor every transition and reuse the models when direction reverses.
    var installed = false
    var lookups = 0
    let lookup: (Locale.Language, Locale.Language) async -> LanguageAvailability.Status = { _, _ in
        lookups += 1
        return installed ? .installed : .supported
    }
    check(await translationPairStatus(source: en, target: zh, lookup: lookup) == .supported,
        "first use permits a genuinely missing pair")
    installed = true
    check(await translationPairStatus(source: en, target: zh, lookup: lookup) == .installed,
        "completed download is reused")
    check(await translationPairStatus(source: zh, target: en, lookup: lookup) == .installed,
        "reverse translation reuses the same assets")
    check(await translationPairStatus(source: en, target: zh, lookup: lookup) == .installed,
        "repeat request stays installed")
    installed = false
    check(await translationPairStatus(source: en, target: zh, lookup: lookup) == .supported,
        "OS removal cannot be hidden by a permanent downloaded flag")
    check(lookups == 7, "installed requests do not enumerate every language or poll")

    let reverseInstalled = await translationPairStatus(source: zh, target: en) { source, _ in
        source == en ? .installed : .supported
    }
    check(reverseInstalled == .installed, "stale forward state reuses installed reverse pair")
    var unsupportedCalls = 0
    let unsupported = await translationPairStatus(source: en, target: zh) { _, _ in
        unsupportedCalls += 1
        return .unsupported
    }
    check(unsupported == .unsupported && unsupportedCalls == 1,
        "unsupported pairs cannot be made supported by reversing them")
    check(!canRequestTranslationDownload([.spanish: 0.28, .portuguese: 0.21]),
        "ambiguous Error selection cannot download Spanish")
    check(!canRequestTranslationDownload([.french: 0.66, .english: 0.14]),
        "ambiguous cache selection cannot download French")
    check(!canRequestTranslationDownload([.indonesian: 0.32, .english: 0.26]),
        "command selection cannot download Indonesian")
    check(!canRequestTranslationDownload([:]), "no detector evidence cannot download")
    check(!canRequestTranslationDownload([.english: 0.85, .french: 0.70]),
        "conflicting hypotheses cannot download")
    check(canRequestTranslationDownload([.english: 0.95, .indonesian: 0.02]),
        "confident English allows first-time installation")
    check(canRequestTranslationDownload([.simplifiedChinese: 1.0]),
        "confident Chinese allows first-time installation")
    print("Translation models: \(checks) regression checks passed")

    #if compiler(>=6.2)
    if CommandLine.arguments.contains("--live"), #available(macOS 26.0, *) {
        let availability = LanguageAvailability()
        let languages = await availability.supportedLanguages
        guard let source = canonicalTranslationLanguage(en, supported: languages),
              let target = canonicalTranslationLanguage(zh, supported: languages) else {
            throw NativeError.message("Live check requires supported English and Simplified Chinese.")
        }
        for (from, to, text) in [(source, target, "Please keep the existing data."),
                                  (target, source, "请保留现有数据。"),
                                  (source, target, "Allow the user to retry.")] {
            let session = TranslationSession(installedSource: from, target: to)
            check(!session.canRequestDownloads, "live session cannot request a download")
            let status = await availability.status(from: from, to: to)
            let reverse = await availability.status(from: to, to: from)
            let ready = await session.isReady
            print("Live metadata: \(from.minimalIdentifier) -> \(to.minimalIdentifier); forward=\(status) reverse=\(reverse) ready=\(ready)")
            // Calling this session remains safe when isReady lags: its type
            // cannot request downloads and returns the actual framework error.
            let result = try await translateBatch([text], using: session)
            check(result.count == 1 && !result[0].isEmpty, "live translation returns a complete result")
            print("Live installed models: \(from.minimalIdentifier) -> \(to.minimalIdentifier) passed; download-capable=false")
        }
    }
    #endif
}

if #available(macOS 15.0, *) {
    Task { @MainActor in
        do { try await runTranslationModelTests(); exit(0) }
        catch { print("Translation model check failed: \(error)"); exit(1) }
    }
    RunLoop.main.run()
} else { fatalError("Translation model checks require macOS 15+.") }
#else
fatalError("Translation model checks require an Apple Translation SDK.")
#endif
