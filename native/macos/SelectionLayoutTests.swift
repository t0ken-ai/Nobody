// Appended to Native.swift by scripts/test-selection-layout.mjs so tests exercise
// its private pure helpers without exporting them through the production bridge.
// No AX calls, windows, credentials, or translation providers are used here.
private func runSelectionLayoutTests() {
    func check(_ actual: String?, _ expected: String?, _ name: String) {
        precondition(actual == expected, "Selection layout regression: \(name)")
    }
    check(mergingSelectionLayout("First.Second.", "First.\n\nSecond."), "First.\n\nSecond.", "adjacent paragraphs")
    check(mergingSelectionLayout("First.\nSecond.", "First.Second."), "First.\nSecond.", "never remove existing breaks")
    check(mergingSelectionLayout("Keep data. Allow retrying.", "Keep data. Allow retrying."), "Keep data. Allow retrying.", "soft wrapping without paragraph evidence")
    check(mergingSelectionLayout("Keep data.", "Keep data.\nAnother selection."), nil, "out-of-selection content")
    check(mergingSelectionLayout("Two words", "Twowords"), nil, "spaces cannot disappear")
    check(mergingSelectionLayout("A  B", "A B"), nil, "indentation is immutable")
    check(mergingSelectionLayout("First.Second.", "First.\u{2029}Second."), "First.\n\nSecond.", "Unicode paragraph marker")
    check(mergingSelectionLayout("A\r\n\r\nB", "A\nB"), "A\r\n\r\nB", "CRLF byte preservation")
    check(insertingSelectionBreaks("😀First.Second.", [8: 2]), "😀First.\n\nSecond.", "UTF-16 offset after emoji")
    check(insertingSelectionBreaks("😀First.", [1: 2]), nil, "reject split surrogate")
    check(insertingSelectionBreaks("e\u{301}First.", [1: 2]), nil, "reject split combining grapheme")
    check(insertingSelectionBreaks("👩‍💻First.", [2: 2]), nil, "reject split joined emoji")
    check(mergingSelectionLayout("👩‍💻First.", "👩\n‍💻First."), nil, "rich text cannot split joined emoji")
    check(mergingSelectionLayout("e\u{301}First.", "e\n\u{301}First."), nil, "rich text cannot split accent")
    check(insertingSelectionBreaks("First.\n\nSecond.", [6: 2, 8: 2]), "First.\n\nSecond.", "no doubled paragraph gaps")
    check(insertingSelectionBreaks("First.Second.", [-1: 2]), nil, "invalid negative range")
    check(insertingSelectionBreaks("First.Second.", [99: 2]), nil, "invalid oversized range")
    print("Selection layout: 17 regression checks passed")
}
runSelectionLayoutTests()
