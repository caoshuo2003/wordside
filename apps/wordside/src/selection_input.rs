//! Shared input policy for automatic capture, shortcuts and manual lookup.
//! Strip what a selection caught around the word (symbols, Chinese, footnote marks);
//! never salvage a word from a path/log/command.
fn whitespace(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// Chinese, Japanese and Korean script and fullwidth forms: text next to an English word,
/// never part of it.
fn cjk(c: char) -> bool {
    matches!(c, '\u{2e80}'..='\u{9fff}' | '\u{ac00}'..='\u{d7af}' | '\u{f900}'..='\u{faff}' | '\u{ff00}'..='\u{ffef}')
}

/// Can go from a word's edges: anything but a letter or an ASCII digit. Letters such as
/// the é of café stay, so a foreign word is turned down instead of cut short.
fn stray(c: char) -> bool {
    !c.is_alphanumeric() || (!c.is_ascii() && (c.is_numeric() || cjk(c)))
}

pub fn selected_word(raw: &str) -> Option<String> {
    let raw = raw.trim_matches(whitespace);
    if raw.chars().any(char::is_control) || raw.chars().count() > 100 || raw.contains(['/', '\\']) {
        return None;
    }
    // A selection that ran past a full stop or comma into a letter or two of the next
    // sentence (`Dictionary. T`) keeps the one clause that has a word in it.
    let clauses = clauses(raw);
    let words: Vec<&String> = clauses.iter().filter(|clause| !fragment(clause)).collect();
    let raw = match words[..] {
        [word] if clauses.len() > 1 => word.as_str(),
        _ => raw,
    };
    // Digits stay at first, since some words have them (mp3, COVID-19); a footnote mark
    // like `record[1]` leaves one behind its bracket, so then they go too.
    trimmed(raw, stray).or_else(|| trimmed(raw, |c| stray(c) || c.is_ascii_digit()))
}

/// The selection cut after each word that ends a sentence or clause, quotes aside.
fn clauses(raw: &str) -> Vec<String> {
    const STOPS: &str = ".,;:!?。，；：！？…";
    const CLOSERS: &str = "\"'”’)]}】」』》〉";
    let mut clauses = vec![String::new()];
    for part in raw.split(whitespace).filter(|part| !part.is_empty()) {
        let clause = clauses.last_mut().unwrap();
        if !clause.is_empty() {
            clause.push(' ');
        }
        clause.push_str(part);
        if part
            .trim_end_matches(|c| CLOSERS.contains(c))
            .ends_with(|c| STOPS.contains(c))
        {
            clauses.push(String::new());
        }
    }
    clauses.retain(|clause| !clause.is_empty());
    clauses
}

/// Nothing a word could be made of but a letter or two, digits or symbols.
fn fragment(clause: &str) -> bool {
    let letters = clause.trim_matches(|c: char| whitespace(c) || stray(c) || c.is_ascii_digit());
    letters.len() <= 2 && letters.bytes().all(|c| c.is_ascii_alphabetic())
}

fn trimmed(raw: &str, edge: impl Fn(char) -> bool) -> Option<String> {
    let word = raw.trim_matches(|c: char| whitespace(c) || edge(c));
    let word = word
        .replace(['‘', '’'], "'")
        .replace(['\u{2010}', '\u{2011}'], "-")
        .split(whitespace)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if word.is_empty()
        || word.len() > 80
        || word.split(' ').count() > 5
        || !word.bytes().any(|c| c.is_ascii_alphabetic())
        || !word
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b' ' | b'\'' | b'-'))
        // Options of a command, like `rm -rf`.
        || word
            .split(' ')
            .any(|part| part.starts_with('-') || part.ends_with('-'))
    {
        return None;
    }
    Some(word)
}

/// What else a selection could have meant, to try in order when the dictionary doesn't
/// have `word`: without digits at its ends (`record2`), then without a letter or two of
/// the neighbouring words (`e record`, `record a`).
pub fn fallback_words(word: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut add = |candidate: Option<String>| {
        if let Some(candidate) = candidate {
            if candidate != word && !words.contains(&candidate) {
                words.push(candidate);
            }
        }
    };
    let digitless = selected_word(
        word.trim_matches(|c: char| c.is_ascii_digit() || matches!(c, ' ' | '-' | '\'')),
    );
    add(digitless.clone());
    let base = digitless.unwrap_or_else(|| word.to_owned());
    let parts: Vec<&str> = base.split(' ').collect();
    let fragment = |part: &str| part.len() <= 2;
    let start = usize::from(fragment(parts[0]));
    let end = parts.len() - usize::from(parts.len() > 1 && fragment(parts[parts.len() - 1]));
    if parts.len() > 1 && start < end {
        add(selected_word(&parts[start..end].join(" ")));
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_cases() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/selection-cases.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let raw = case["raw"].as_str().unwrap();
            let word = selected_word(raw);
            assert_eq!(word.as_deref(), case["expected"].as_str(), "{raw:?}");
            if let Some(word) = word {
                assert_eq!(
                    selected_word(&word).as_deref(),
                    Some(word.as_str()),
                    "normalization must be stable: {raw:?}"
                );
            }
        }
    }

    #[test]
    fn fallbacks_drop_neighbouring_digits_and_fragments_only() {
        assert_eq!(fallback_words("record2"), ["record"]);
        assert_eq!(fallback_words("1 record"), ["record"]);
        assert_eq!(fallback_words("e record"), ["record"]);
        assert_eq!(fallback_words("record a"), ["record"]);
        assert_eq!(fallback_words("he record t"), ["record"]);
        assert_eq!(fallback_words("mp3"), ["mp"]);
        for word in ["record", "go on", "state-of-the-art", "don't"] {
            assert!(fallback_words(word).is_empty(), "{word}");
        }
    }
}
