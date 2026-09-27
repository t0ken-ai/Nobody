//! System translation receives prose only and restores protected spans locally.
//! LLM translation sees the full document for context; these same spans form a
//! minimum integrity check in addition to the model's inferred code regions.
use regex::Regex;
use std::sync::LazyLock;

static PROTECTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?ms)```[^\n]*\n.*?(?:^```[^\n]*$|\z)|~~~[^\n]*\n.*?(?:^~~~[^\n]*$|\z)|`+[^`\n]*`+|https?://[^\s<>]+|(?:[A-Za-z]:[\\/]|(?:\.{0,2}|~)/)[A-Za-z0-9_.@/\\-]+|\b(?:[A-Za-z0-9_.@-]+[\\/])+[A-Za-z0-9_.@/\\-]+|\b[A-Za-z_][A-Za-z0-9]*_[A-Za-z0-9_]+\b|\b[a-z]+(?:[A-Z][A-Za-z0-9]*)+\b|\b[A-Z][A-Z0-9_]{2,}\b|\b[A-Za-z0-9_-]+\.(?:rs|ts|tsx|js|jsx|py|swift|json|toml|md|yaml|yml|sh|css|html|cpp|h)\b"
).expect("valid code protection pattern")
});

pub struct Document {
    parts: Vec<(bool, String)>,
}
impl Document {
    /// Whitespace is retained outside translation segments, keeping paragraphs,
    /// Markdown fences and code byte-for-byte unchanged across both providers.
    pub fn parse(input: &str) -> Self {
        let mut parts = Vec::new();
        let mut cursor = 0;
        for found in PROTECTED.find_iter(input) {
            // These are prose/product terms, not variable names. Splitting at
            // "macOS" made the real chat sample translate into two broken
            // clauses. Explicit backticks still protect any of these terms.
            if [
                "macOS",
                "iOS",
                "iPadOS",
                "watchOS",
                "visionOS",
                "tvOS",
                "JavaScript",
                "TypeScript",
                "OpenAI",
                "GitHub",
                "GitLab",
                "ChatGPT",
                "LLM",
                "API",
                "HTTP",
                "HTTPS",
                "JSON",
                "HTML",
                "CSS",
                "SQL",
                "URL",
                "SDK",
            ]
            .contains(&found.as_str())
            {
                continue;
            }
            split_prose(&input[cursor..found.start()], &mut parts);
            parts.push((false, found.as_str().into()));
            cursor = found.end();
        }
        split_prose(&input[cursor..], &mut parts);
        Self { parts }
    }
    pub fn prose(&self) -> Vec<String> {
        self.parts
            .iter()
            .filter(|(translate, _)| *translate)
            .map(|(_, text)| text.clone())
            .collect()
    }
    /// Known code, paths and identifiers must survive full-context translation.
    /// Whitespace-only fragments are layout, not immutable source literals.
    pub fn protected_spans(&self) -> Vec<&str> {
        self.parts
            .iter()
            .filter(|(translate, text)| !translate && text.chars().any(char::is_alphanumeric))
            .map(|(_, text)| text.as_str())
            .collect()
    }
    pub fn restore(&self, translated: Vec<String>) -> Result<String, String> {
        if translated.len() != self.parts.iter().filter(|(t, _)| *t).count() {
            return Err("翻译结果的段落数不匹配，已保留原文。".into());
        }
        let mut translated = translated.into_iter();
        let mut result = String::new();
        for (translate, original) in &self.parts {
            if *translate {
                let text = translated.next().unwrap();
                if text.trim().is_empty() {
                    return Err("服务返回空译文，已保留原文。".into());
                }
                result.push_str(text.trim());
            } else {
                result.push_str(original);
            }
        }
        Ok(result)
    }
}

fn split_prose(text: &str, parts: &mut Vec<(bool, String)>) {
    for line in text.split_inclusive('\n') {
        let start = line.len() - line.trim_start().len();
        let end = line.trim_end().len();
        if start >= end || !line[start..end].chars().any(char::is_alphabetic) {
            if !line.is_empty() {
                parts.push((false, line.into()));
            }
            continue;
        }
        if start > 0 {
            parts.push((false, line[..start].into()));
        }
        parts.push((true, line[start..end].into()));
        if end < line.len() {
            parts.push((false, line[end..].into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn code_paths_and_line_breaks_round_trip_without_model_access() {
        let input = "请修改 `user_id` 和 src/main.rs：\n\n```rust\nlet 用户 = 1;\n```\n访问 https://example.com/api?a=1\n";
        let d = Document::parse(input);
        assert!(d
            .prose()
            .iter()
            .all(|p| !p.contains("user_id") && !p.contains("用户") && !p.contains("https://")));
        assert_eq!(d.restore(d.prose()).unwrap(), input);
    }
    #[test]
    fn incomplete_fence_and_malformed_provider_output_are_safe() {
        let d = Document::parse("翻译一下\n```sh\nrm -rf /tmp/example");
        assert_eq!(d.prose(), vec!["翻译一下"]);
        assert!(d.restore(vec![]).is_err());
        assert!(d.restore(vec![String::new()]).is_err());
    }
    #[test]
    fn preserves_unicode_padding_and_identifiers() {
        let input = "　修复 getUserName 中的 MAX_RETRY 和 user_id。\n";
        let d = Document::parse(input);
        assert_eq!(d.restore(d.prose()).unwrap(), input);
    }
    #[test]
    fn platform_names_do_not_break_sentence_context() {
        let input = "我想写一个 macOS 和 Windows 都能用的翻译器，支持 LLM API。";
        assert_eq!(Document::parse(input).prose(), vec![input]);
        assert!(Document::parse("保留 `macOS`")
            .prose()
            .iter()
            .all(|s| !s.contains("macOS")));
    }
    #[test]
    fn relative_paths_are_protected_including_the_first_directory() {
        // Matching only "/main.rs" leaked "src" into the model and could
        // corrupt a path even though its filename appeared to be protected.
        for path in [
            "src/main.rs",
            "config/settings.json",
            "../lib/app.ts",
            r"C:\Projects\app.rs",
            r"src\main.rs",
        ] {
            let d = Document::parse(&format!("修改 {path} 然后测试"));
            assert_eq!(d.prose(), vec!["修改", "然后测试"]);
        }
    }
}
