//! Shared input policy for automatic capture, shortcuts and manual lookup.
//! Strip surrounding punctuation only; never salvage a word from a path/log/command.
fn whitespace(c: char) -> bool { (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}' }
pub fn selected_word(raw: &str) -> Option<String> {
    let raw = raw.trim_matches(whitespace);
    if raw.chars().any(char::is_control) || raw.chars().count() > 100 { return None; }
    const EDGES: &str = "`\"'.,;:!?()[]{}“”‘’，。；：！？（）【】《》〈〉「」『』…·•-‐‑‒–—―−﹘﹣－_=+*|#$%^&<>";
    let word = raw.trim_matches(|c: char| whitespace(c) || EDGES.contains(c));
    let word = word.replace(['‘', '’'], "'").replace(['\u{2010}', '\u{2011}'], "-").split(whitespace).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    if word.is_empty() || word.len() > 80 || word.split(' ').count() > 5 ||
        !word.bytes().any(|c| c.is_ascii_alphabetic()) ||
        !word.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b' ' | b'\'' | b'-')) {
        return None;
    }
    Some(word)
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn shared_selection_cases() {
        let cases: serde_json::Value = serde_json::from_str(include_str!("../../../shared/tests/selection-cases.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let raw = case["raw"].as_str().unwrap();
            let word = selected_word(raw);
            assert_eq!(word.as_deref(), case["expected"].as_str(), "{raw:?}");
            if let Some(word) = word { assert_eq!(selected_word(&word).as_deref(), Some(word.as_str()), "normalization must be stable: {raw:?}"); }
        }
    }
}
