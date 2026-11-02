//! Validates the dictionary JSON before anything is shown or cached.
//! Ported from `shared/dictionary.ts`; the rules and their tests are unchanged.
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Example {
    pub text: String,
    pub translation: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Definition {
    pub pos: String,
    pub source: String,
    pub text: String,
    pub translation: String,
    pub examples: Vec<Example>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pronunciation {
    pub pos: String,
    pub lang: String,
    pub pron: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub word: String,
    pub query: String,
    pub english_only_source: bool,
    pub source_url: String,
    pub pos: Vec<String>,
    pub definitions: Vec<Definition>,
    pub pronunciation: Vec<Pronunciation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LookupError {
    pub not_found: bool,
    pub suggestions: Vec<String>,
    pub code: String,
    pub status: Option<u64>,
}

impl LookupError {
    pub fn invalid() -> Self {
        Self {
            not_found: false,
            suggestions: Vec::new(),
            code: "INVALID_RESPONSE".into(),
            status: None,
        }
    }
    pub fn other(code: &str) -> Self {
        Self {
            not_found: false,
            suggestions: Vec::new(),
            code: code.into(),
            status: None,
        }
    }
}

type Checked<T> = Result<T, LookupError>;

pub fn identity(value: &str) -> String {
    let lowered = value.to_lowercase();
    let collapsed = lowered
        .split(' ')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let key = collapsed.trim().replace('’', "'");
    if key == "runtime" {
        "run-time".into()
    } else {
        key
    }
}

pub fn combine_suggestions(query: &str, groups: &[&[String]]) -> Vec<String> {
    let mut seen = vec![identity(query)];
    let mut result = Vec::new();
    for word in groups.iter().flat_map(|group| group.iter()) {
        let key = identity(word);
        if !seen.contains(&key) && result.len() < 10 {
            seen.push(key);
            result.push(word.clone());
        }
    }
    result
}

fn valid_candidate(word: &str) -> bool {
    let mut chars = word.chars();
    !word.is_empty()
        && word.chars().count() <= 80
        && word.trim() == word
        && word.split(' ').count() <= 5
        && word.chars().any(|c| c.is_ascii_alphabetic())
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '\'' | '-'))
}

/// `None` means the field was absent, which is not the same as a malformed value.
pub fn read_suggestions(value: Option<&Value>, query: &str) -> Checked<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let list = value
        .as_array()
        .filter(|list| list.len() <= 10)
        .ok_or_else(LookupError::invalid)?;
    let mut words = Vec::new();
    for word in list {
        let word = word
            .as_str()
            .filter(|word| valid_candidate(word))
            .ok_or_else(LookupError::invalid)?;
        words.push(word.to_owned());
    }
    Ok(combine_suggestions(query, &[&words]))
}

fn text(value: Option<&Value>) -> Checked<String> {
    Ok(value
        .and_then(Value::as_str)
        .ok_or_else(LookupError::invalid)?
        .to_owned())
}

fn optional_text(value: Option<&Value>) -> Checked<String> {
    if value.is_none() {
        Ok(String::new())
    } else {
        text(value)
    }
}

fn list(value: Option<&Value>) -> Checked<&Vec<Value>> {
    value
        .and_then(Value::as_array)
        .ok_or_else(LookupError::invalid)
}

fn check_id(value: Option<&Value>) -> Checked<()> {
    match value {
        None => Ok(()),
        Some(id) if id.is_u64() => Ok(()),
        Some(id) if id.as_f64().is_some_and(|n| n >= 0.0 && n.fract() == 0.0) => Ok(()),
        Some(_) => Err(LookupError::invalid()),
    }
}

fn safe_url(value: &str) -> String {
    match reqwest::Url::parse(value) {
        Ok(url) if matches!(url.scheme(), "https" | "http") => url.to_string(),
        _ => String::new(),
    }
}

fn read_definition(value: &Value) -> Checked<Definition> {
    let definition = value.as_object().ok_or_else(LookupError::invalid)?;
    let meaning = text(definition.get("text"))?;
    let translation = text(definition.get("translation"))?;
    let mut examples = Vec::new();
    for example in list(definition.get("example"))? {
        let example = example.as_object().ok_or_else(LookupError::invalid)?;
        check_id(example.get("id"))?;
        examples.push(Example {
            text: text(example.get("text"))?,
            translation: text(example.get("translation"))?,
        });
    }
    check_id(definition.get("id"))?;
    Ok(Definition {
        pos: text(definition.get("pos"))?,
        source: optional_text(definition.get("source"))?,
        text: meaning,
        translation,
        examples,
    })
}

pub fn word_url(word: &str, language: &str) -> String {
    let dictionary = if language == "en" {
        "english"
    } else {
        "english-chinese-simplified"
    };
    let mut encoded = String::new();
    for byte in word.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("https://dictionary.cambridge.org/dictionary/{dictionary}/{encoded}")
}

/// Mirrors `readApiEntry`: structured error fields always outrank legacy error text.
pub fn read_api_entry(payload: &Value, query: &str, language: &str) -> Checked<Entry> {
    let status = payload.get("apiStatus").and_then(Value::as_u64);
    let code = payload.get("apiCode").and_then(Value::as_str);
    let typed = payload.get("apiStatus").is_some() || payload.get("apiCode").is_some();
    let api_error = payload
        .get("apiError")
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty());
    if api_error.is_some() || typed {
        let not_found = if typed {
            status == Some(404) && code == Some("WORD_NOT_FOUND")
        } else {
            api_error.is_some_and(|message| message.starts_with("词典服务返回 HTTP 404"))
        };
        let suggestions = if not_found && typed {
            read_suggestions(payload.get("suggestions"), query)?
        } else {
            Vec::new()
        };
        return Err(LookupError {
            not_found,
            suggestions,
            code: code.unwrap_or("LOOKUP_FAILED").into(),
            status,
        });
    }
    let data = payload
        .get("entry")
        .and_then(Value::as_object)
        .ok_or_else(LookupError::invalid)?;
    if let Some(error) = data.get("error").and_then(Value::as_str) {
        let inner = data.get("code");
        let not_found = error.trim().eq_ignore_ascii_case("word not found")
            && (inner.is_none() || inner.and_then(Value::as_str) == Some("WORD_NOT_FOUND"));
        let code = inner.and_then(Value::as_str).unwrap_or("LOOKUP_FAILED");
        return Err(LookupError {
            not_found,
            suggestions: Vec::new(),
            code: code.into(),
            status: None,
        });
    }
    let word = text(data.get("word"))?;
    if word.trim().is_empty() {
        return Err(LookupError::invalid());
    }
    let pos = list(data.get("pos"))?
        .iter()
        .map(|value| text(Some(value)))
        .collect::<Checked<Vec<_>>>()?;
    let definitions = list(data.get("definition"))?
        .iter()
        .map(read_definition)
        .collect::<Checked<Vec<_>>>()?;
    let mut pronunciation = Vec::new();
    for value in list(data.get("pronunciation"))? {
        let item = value.as_object().ok_or_else(LookupError::invalid)?;
        pronunciation.push(Pronunciation {
            pos: text(item.get("pos"))?,
            lang: text(item.get("lang"))?,
            pron: text(item.get("pron"))?,
            url: safe_url(&text(item.get("url"))?),
        });
    }
    let source_url = word_url(&word, language);
    Ok(Entry {
        word,
        query: query.to_owned(),
        english_only_source: language == "en",
        source_url,
        pos,
        definitions,
        pronunciation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn missing(suggestions: Value) -> Value {
        json!({"entry":null,"apiError":"词典服务返回 HTTP 404；未找到此词条","apiStatus":404,"apiCode":"WORD_NOT_FOUND","suggestions":suggestions})
    }
    fn failure(payload: Value) -> LookupError {
        read_api_entry(&payload, "termnal", "en-cn").unwrap_err()
    }
    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn typed_missing_error_retains_candidates_and_status_outranks_legacy_text() {
        let error = failure(missing(json!(["terminal", "terminals"])));
        assert!(error.not_found);
        assert_eq!(error.suggestions, strings(&["terminal", "terminals"]));
        for patch in [("apiStatus", json!(502)), ("apiCode", json!("OTHER_ERROR"))] {
            let mut payload = missing(json!(["terminal"]));
            payload[patch.0] = patch.1;
            let error = failure(payload);
            assert!(!error.not_found && error.suggestions.is_empty());
        }
        let error = failure(json!({"entry":null,"apiError":"词典服务返回 HTTP 404；未找到此词条"}));
        assert!(error.not_found && error.suggestions.is_empty());
    }

    #[test]
    fn legacy_error_text_cannot_override_structured_metadata() {
        for payload in [
            json!({"entry":{"error":"word not found","suggestions":["terminal"]},"apiStatus":502,"apiCode":"CAMBRIDGE_BLOCKED"}),
            json!({"entry":null,"apiError":"词典服务返回 HTTP 404","apiCode":"CAMBRIDGE_BLOCKED","suggestions":["terminal"]}),
            json!({"entry":{"error":"word not found","code":"CAMBRIDGE_BLOCKED","suggestions":["terminal"]}}),
        ] {
            let error = failure(payload);
            assert!(!error.not_found && error.suggestions.is_empty());
        }
        for payload in [
            json!({"entry":{"error":"word not found","suggestions":["terminal"]}}),
            json!({"entry":null,"apiError":"词典服务返回 HTTP 404","suggestions":["terminal"]}),
        ] {
            let error = failure(payload);
            assert!(error.not_found && error.suggestions.is_empty());
        }
    }

    #[test]
    fn candidate_validation_rejects_malformed_oversized_and_non_dictionary_words() {
        for value in [
            json!({}),
            json!(["bad/path"]),
            json!([null]),
            json!(["<img>"]),
            json!(["two\nlines"]),
            json!([" terminal"]),
            json!(["a".repeat(81)]),
            json!(["one two three four five six"]),
            json!(vec!["terminal"; 11]),
        ] {
            let error = read_suggestions(Some(&value), "termnal").unwrap_err();
            assert!(!error.not_found, "{value}");
        }
        assert!(read_suggestions(None, "termnal").unwrap().is_empty());
    }

    #[test]
    fn candidate_lists_preserve_order_while_excluding_the_query_and_duplicates() {
        let first = strings(&["termnal", "terminal", "terminals"]);
        let second = strings(&["Terminal", "eternal"]);
        assert_eq!(
            combine_suggestions("termnal", &[&first, &second]),
            strings(&["terminal", "terminals", "eternal"])
        );
        let many: Vec<String> = (0..20).map(|i| format!("candidate{i}")).collect();
        assert_eq!(combine_suggestions("query", &[&many]).len(), 10);
    }

    #[test]
    fn successful_entries_ignore_suggestion_fields_and_keep_complete_definitions() {
        let entry = json!({"word":"terminal","pos":["noun"],"verbs":[],"pronunciation":[],"definition":[{"text":"Synthetic meaning","translation":"合成释义","pos":"noun","source":"TEST","example":[{"text":"Synthetic example","translation":"合成例句"}]}]});
        let result = read_api_entry(
            &json!({"entry":entry,"suggestions":["other"]}),
            "terminal",
            "en-cn",
        )
        .unwrap();
        assert_eq!(result.word, "terminal");
        assert_eq!(result.definitions.len(), 1);
        assert_eq!(result.definitions[0].examples[0].translation, "合成例句");
        assert_eq!(result.definitions[0].source, "TEST");
    }

    #[test]
    fn malformed_entries_are_rejected_and_unsafe_audio_urls_are_blanked() {
        for payload in [
            json!({"entry":{}}),
            json!({"entry":{"word":"terminal","pos":[],"definition":"bad","pronunciation":[]}}),
            json!({"entry":{"word":"  ","pos":[],"definition":[],"pronunciation":[]}}),
            json!({"entry":{"word":"a","pos":[],"definition":[{"id":-1,"text":"","translation":"","pos":"","example":[]}],"pronunciation":[]}}),
        ] {
            assert_eq!(failure(payload).code, "INVALID_RESPONSE");
        }
        let entry = json!({"entry":{"word":"a","pos":[],"definition":[],"pronunciation":[{"pos":"","lang":"uk","pron":"/a/","url":"javascript:alert(1)"}]}});
        assert_eq!(
            read_api_entry(&entry, "a", "en-cn").unwrap().pronunciation[0].url,
            ""
        );
    }

    #[test]
    fn every_parser_fixture_passes_validation() {
        let fixtures: Value =
            serde_json::from_str(include_str!("../tests/fixtures/native-cambridge.json")).unwrap();
        let mut checked = 0;
        for page in fixtures["pages"].as_array().unwrap() {
            if page["entry"].is_object() {
                let entry =
                    read_api_entry(&json!({"entry":page["entry"]}), "example", "en-cn").unwrap();
                assert_eq!(
                    entry.definitions.len(),
                    page["entry"]["definition"].as_array().unwrap().len()
                );
                checked += 1;
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn source_links_are_encoded_for_each_dictionary() {
        assert_eq!(
            word_url("run on", "en"),
            "https://dictionary.cambridge.org/dictionary/english/run%20on"
        );
        assert_eq!(
            word_url("don't", "en-cn"),
            "https://dictionary.cambridge.org/dictionary/english-chinese-simplified/don't"
        );
    }
}
