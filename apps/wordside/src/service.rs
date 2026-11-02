//! One lookup from a typed word to a validated entry: cache, per-word gate, English fallback.
//! Replaces the orchestration that 0.2.0 split between `main.rs` and `app.ts`.
use crate::{
    cache::Cache,
    dictionary::Backend,
    entry::{combine_suggestions, read_api_entry, Entry, LookupError},
    lookup::fetch_payload,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};

const LOOKUP_LIMIT: Duration = Duration::from_secs(35);

#[derive(Default)]
pub struct Service {
    backend: Backend,
    cache: Mutex<Cache>,
    gates: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    tickets: AtomicU64,
}

impl Service {
    /// The caller cancels by aborting the task, which also drops the gate and any network wait.
    pub async fn lookup(&self, word: &str, refresh: bool) -> Result<Entry, LookupError> {
        let job = resolve(word, |language| self.fetch(word, language, refresh));
        tokio::time::timeout(LOOKUP_LIMIT, job)
            .await
            .unwrap_or_else(|_| Err(LookupError::other("CAMBRIDGE_TIMEOUT")))
    }

    fn gate(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut gates = self.gates.lock().unwrap();
        gates.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = gates.get(key).and_then(Weak::upgrade) {
            return gate;
        }
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        gates.insert(key.to_owned(), Arc::downgrade(&gate));
        gate
    }

    async fn fetch(
        &self,
        word: &str,
        language: &'static str,
        refresh: bool,
    ) -> Result<Entry, LookupError> {
        let key = json!([word.to_lowercase(), language]).to_string();
        let gate = self.gate(&key);
        let _guard = gate.lock().await;
        if !refresh {
            let cached = self.cache.lock().unwrap().get(&key);
            if let Some(cached) = cached {
                return read_api_entry(&cached, word, language);
            }
        }
        let payload: Value = fetch_payload(&self.backend, word, language)
            .await
            .map_err(|_| LookupError::other("LOOKUP_FAILED"))?;
        if !payload["entry"].is_object() {
            return read_api_entry(&payload, word, language);
        }
        // Only a complete entry stays cached; a newer response can't be replaced by this one.
        let ticket = self.tickets.fetch_add(1, Ordering::SeqCst) + 1;
        self.cache
            .lock()
            .unwrap()
            .insert_pending(key, payload.clone(), ticket);
        let entry = read_api_entry(&payload, word, language);
        let accepted = entry
            .as_ref()
            .is_ok_and(|entry| !entry.definitions.is_empty());
        self.cache.lock().unwrap().settle(ticket, accepted);
        entry
    }
}

/// English is tried only when the bilingual dictionary clearly lacks the word.
pub async fn resolve<F, Fut>(word: &str, fetch: F) -> Result<Entry, LookupError>
where
    F: Fn(&'static str) -> Fut,
    Fut: Future<Output = Result<Entry, LookupError>>,
{
    let primary = match fetch("en-cn").await {
        Err(primary) if primary.not_found => primary,
        other => return other,
    };
    match fetch("en").await {
        Err(english) if english.not_found => Err(LookupError {
            not_found: true,
            suggestions: combine_suggestions(word, &[&primary.suggestions, &english.suggestions]),
            code: "WORD_NOT_FOUND".into(),
            status: Some(404),
        }),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn entry(word: &str) -> Entry {
        read_api_entry(
            &json!({"entry":{"word":word,"pos":[],"definition":[],"pronunciation":[]}}),
            word,
            "en-cn",
        )
        .unwrap()
    }
    fn missing(words: &[&str]) -> LookupError {
        LookupError {
            not_found: true,
            suggestions: words.iter().map(|w| (*w).to_owned()).collect(),
            code: "WORD_NOT_FOUND".into(),
            status: Some(404),
        }
    }

    async fn run(
        first: Result<Entry, LookupError>,
        second: Result<Entry, LookupError>,
    ) -> (Result<Entry, LookupError>, Vec<&'static str>) {
        let calls = RefCell::new(Vec::new());
        let result = resolve("termnal", |language| {
            calls.borrow_mut().push(language);
            let answer = if language == "en-cn" {
                first.clone()
            } else {
                second.clone()
            };
            async move { answer }
        })
        .await;
        (result, calls.into_inner())
    }

    #[tokio::test]
    async fn found_in_the_bilingual_dictionary_never_asks_english() {
        let (result, calls) = run(Ok(entry("terminal")), Err(missing(&[]))).await;
        assert_eq!(result.unwrap().word, "terminal");
        assert_eq!(calls, ["en-cn"]);
    }

    #[tokio::test]
    async fn only_a_clear_miss_falls_back_to_english() {
        let (result, calls) = run(Err(missing(&[])), Ok(entry("terminal"))).await;
        assert_eq!(result.unwrap().word, "terminal");
        assert_eq!(calls, ["en-cn", "en"]);
        for code in [
            "CAMBRIDGE_BLOCKED",
            "CAMBRIDGE_UNAVAILABLE",
            "CAMBRIDGE_TIMEOUT",
        ] {
            let (result, calls) = run(Err(LookupError::other(code)), Ok(entry("terminal"))).await;
            assert_eq!(result.unwrap_err().code, code);
            assert_eq!(calls, ["en-cn"], "{code} must not fall back");
        }
    }

    #[tokio::test]
    async fn a_miss_in_both_merges_suggestions_without_the_query() {
        let (result, _) = run(
            Err(missing(&["terminal", "termnal"])),
            Err(missing(&["Terminal", "eternal"])),
        )
        .await;
        let error = result.unwrap_err();
        assert!(error.not_found);
        assert_eq!(error.suggestions, ["terminal", "eternal"]);
    }

    #[tokio::test]
    async fn english_failure_after_a_miss_keeps_its_own_error() {
        let (result, _) = run(
            Err(missing(&["terminal"])),
            Err(LookupError::other("CAMBRIDGE_BLOCKED")),
        )
        .await;
        let error = result.unwrap_err();
        assert!(!error.not_found);
        assert_eq!(error.code, "CAMBRIDGE_BLOCKED");
    }
}
