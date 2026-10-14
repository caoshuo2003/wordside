//! Rust port of the Nongfsq-derived parser in modules/cambridge-dictionary-api.
//! Preserve raw descendant text, field names and source order. See UPSTREAM.md.
use super::QueryError;
use dom_query::{Document, Selection};
use regex::Regex;
use reqwest::Url;
use serde_json::{json, Value};
use std::{collections::HashSet, sync::LazyLock};

pub const SITE: &str = "https://dictionary.cambridge.org";
static COMPLETE: LazyLock<Regex> = LazyLock::new(|| {
    let ws = r"[\t\n\x0B\f\r \u{00A0}\u{FEFF}\u{1680}\u{2000}-\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}]";
    Regex::new(&format!(r"(?is)</html{ws}*>{ws}*(?:<!--[\s\S]*?-->{ws}*)*$")).unwrap()
});
static LANDING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https://dictionary\.cambridge\.org/(?:us/|uk/)?dictionary/english(?:-chinese-simplified|-chinese-traditional)?/$").unwrap());

fn document(html: &str) -> Result<Document, QueryError> {
    if !COMPLETE.is_match(html) { return Err(QueryError::invalid_page()); }
    let doc = Document::from(html);
    let title = doc.select("title").text().to_lowercase();
    if doc.select("#challenge-form, #cf-chl-widget, #cf-challenge-running").exists() ||
        ["just a moment", "access denied", "verify you are human"].iter().any(|s| title.contains(s)) {
        return Err(QueryError::blocked());
    }
    Ok(doc)
}
fn closest<'a>(selection: &Selection<'a>, selector: &str) -> Selection<'a> {
    let mut current = selection.clone();
    while current.exists() && !current.is(selector) { current = current.parent(); }
    current
}
pub fn parse_page(html: &str) -> Result<Value, QueryError> {
    let doc = document(html)?;
    let word = doc.select_single(".hw.dhw").text().to_string();
    if word.is_empty() {
        let canonical = doc.select("link[rel=canonical]").attr_or("href", "");
        return Err(if LANDING.is_match(&canonical) { QueryError::not_found() } else { QueryError::invalid_page() });
    }
    let mut seen = HashSet::new();
    let pos: Vec<String> = doc.select(".pos.dpos").iter().map(|p| p.text().to_string()).filter(|p| seen.insert(p.clone())).collect();
    let mut pronunciation = Vec::new();
    for header in doc.select(".pos-header.dpos-h").iter() {
        let pos_node = header.select_single(".dpos-g");
        if !pos_node.exists() { continue; }
        let p = pos_node.text();
        for node in header.select(".dpron-i").iter() {
            let lang = node.select(".region.dreg").text();
            let audio = node.select("audio source").attr_or("src", "");
            let pron = node.select(".pron.dpron").text();
            if !audio.is_empty() && !pron.is_empty() {
                pronunciation.push(json!({"pos":p.as_ref(),"lang":lang.as_ref(),"url":format!("{SITE}{audio}"),"pron":pron.as_ref()}));
            }
        }
    }
    let blocks = doc.select(".def-block.ddef_block");
    if blocks.is_empty() { return Err(QueryError::invalid_page()); }
    let mut definition = Vec::with_capacity(blocks.length());
    let mut assigned_bodies = 0;
    for (index, block) in blocks.iter().enumerate() {
        let pos = closest(&block, ".pr.entry-body__el").select_single(".pos.dpos").text();
        let source = closest(&block, ".pr.dictionary").attr("data-id");
        let texts = block.select(".def.ddef_d.db");
        let text = texts.text();
        let translation = block.select(".def-body.ddef_b > span.trans.dtrans").text();
        let body_count = block.select(".def-body.ddef_b").length();
        if texts.length() != 1 || body_count > 1 || text.trim_matches(js_whitespace).is_empty() ||
            (body_count == 0 && block.select(".examp.dexamp").exists()) ||
            block.select(".trans.dtrans").length() != block.select(".def-body.ddef_b .trans.dtrans").length() {
            return Err(QueryError::invalid_page());
        }
        assigned_bodies += body_count;
        let examples = block.select(".def-body.ddef_b > .examp.dexamp");
        if examples.length() != block.select(".examp.dexamp").length() { return Err(QueryError::invalid_page()); }
        let mut example = Vec::with_capacity(examples.length());
        for (id, ex) in examples.iter().enumerate() {
            let texts = ex.select(".eg.deg");
            let text = texts.text();
            if texts.length() != 1 || text.trim_matches(js_whitespace).is_empty() { return Err(QueryError::invalid_page()); }
            example.push(json!({"id":id,"text":text.as_ref(),"translation":ex.select(".trans.dtrans").text().as_ref()}));
        }
        let mut value = json!({"id":index,"pos":pos.as_ref(),"text":text.as_ref(),"translation":translation.as_ref(),"example":example});
        // JavaScript omitted undefined data-id; keep that distinction from null.
        if let Some(source) = source { value["source"] = json!(source.as_ref()); }
        definition.push(value);
    }
    if doc.select(".def.ddef_d.db").length() != definition.len() || doc.select(".def-body.ddef_b").length() != assigned_bodies {
        return Err(QueryError::invalid_page());
    }
    Ok(json!({"word":word,"pos":pos,"verbs":[],"pronunciation":pronunciation,"definition":definition}))
}

fn dataset(language: &str) -> Result<(&'static str, &'static str), QueryError> {
    match language {
        "en" => Ok(("us", "english")), "uk" => Ok(("uk", "english")),
        "en-cn" => Ok(("us", "english-chinese-simplified")), "en-tw" => Ok(("us", "english-chinese-traditional")),
        _ => Err(QueryError::new(400, "UNSUPPORTED_LANGUAGE")),
    }
}
pub fn dictionary_url(language: &str, entry: &str) -> Result<Url, QueryError> {
    let (nation, dataset) = dataset(language)?;
    if entry.trim_matches(js_whitespace).is_empty() || entry.encode_utf16().count() > 120 ||
        entry.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}' || matches!(c, '/' | '\\')) || matches!(entry.trim_matches(js_whitespace), "." | "..") {
        return Err(QueryError::new(400, "INVALID_ENTRY"));
    }
    // Match JavaScript encodeURIComponent, including punctuation in diagnostic inputs.
    let encoded = entry.trim_matches(js_whitespace).bytes().map(|byte| {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) { (byte as char).to_string() }
        else { format!("%{byte:02X}") }
    }).collect::<String>();
    Ok(Url::parse(&format!("{SITE}/{nation}/dictionary/{dataset}/{encoded}")).unwrap())
}
pub fn suggestion_url(language: &str, entry: &str) -> Result<Url, QueryError> {
    let mut url = dictionary_url(language, entry)?;
    let (nation, dataset) = dataset(language)?;
    url.set_path(&format!("/{nation}/spellcheck/{dataset}/"));
    url.query_pairs_mut().append_pair("q", entry.trim_matches(js_whitespace));
    Ok(url)
}
fn js_whitespace(c: char) -> bool {
    matches!(c, '\u{9}'..='\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}
fn normalized(word: &str) -> String { word.replace('’', "'").split(js_whitespace).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ") }
fn same_origin(url: &Url) -> bool {
    url.scheme() == "https" && url.host_str() == Some("dictionary.cambridge.org") &&
        url.port_or_known_default() == Some(443) && url.username().is_empty() && url.password().is_none()
}
fn allowed_path(url: &Url, expected: &str, language: &str) -> bool {
    url.path() == expected || (language == "uk" && url.path() == expected.replacen("/uk/", "/", 1))
}
pub fn parse_suggestions(html: &str, language: &str, entry: &str) -> Result<Vec<String>, QueryError> {
    let expected = suggestion_url(language, entry)?;
    let doc = document(html)?;
    let source = doc.select("meta[property='og:url']").attr_or("content", "");
    let source = Url::parse(&source).map_err(|_| QueryError::invalid_page())?;
    let query = normalized(entry).to_lowercase();
    if !same_origin(&source) || !allowed_path(&source, expected.path(), language) ||
        normalized(&doc.select("#searchword").attr_or("value", "")).to_lowercase() != query {
        return Err(QueryError::invalid_page());
    }
    let headings = doc.select("h1");
    let heading = headings.iter().find(|h| h.text().strip_prefix("Search suggestions for").is_some_and(|tail| tail.chars().next().is_some_and(js_whitespace)));
    let Some(heading) = heading else {
        return if headings.text().trim_matches(js_whitespace) == "Your search terms did not match any entries." { Ok(Vec::new()) } else { Err(QueryError::invalid_page()) };
    };
    if normalized(&heading.select("span.tb").text()).to_lowercase() != query { return Err(QueryError::invalid_page()); }
    let mut list = heading.next_sibling();
    while list.exists() && !list.is("ul.hul-u") { list = list.next_sibling(); }
    if list.is_empty() { return Err(QueryError::invalid_page()); }
    let expected_path = expected.path().replace("/spellcheck/", "/search/") + "direct/";
    let mut seen = HashSet::from([query]);
    let mut words = Vec::new();
    for link in list.children().filter("li").children().filter("a[href]").iter() {
        let href = link.attr_or("href", "");
        let Ok(url) = Url::parse(SITE).unwrap().join(&href) else { continue; };
        if !same_origin(&url) || !allowed_path(&url, &expected_path, language) { continue; }
        let values = url.query_pairs().filter(|(key, _)| key == "q").map(|(_, value)| value.into_owned()).collect::<Vec<_>>();
        if values.len() != 1 { continue; }
        let word = normalized(&link.text());
        let key = word.to_lowercase();
        if word.is_empty() || word.len() > 80 || word.split(' ').count() > 5 ||
            !word.as_bytes()[0].is_ascii_alphanumeric() || !word.bytes().any(|c| c.is_ascii_alphabetic()) ||
            !word.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b' ' | b'\'' | b'-')) ||
            normalized(&values[0]).to_lowercase() != key || !seen.insert(key) { continue; }
        words.push(word);
        if words.len() == 10 { break; }
    }
    Ok(words)
}

#[cfg(test)] mod tests {
    use super::*;
    fn cases() -> Value { serde_json::from_str(include_str!("../../tests/fixtures/native-cambridge.json")).unwrap() }
    #[test] fn entry_json_matches_original_oracle_and_rejects_incomplete_content() {
        for case in cases()["pages"].as_array().unwrap() {
            let result = parse_page(case["html"].as_str().unwrap());
            if case.get("entry").is_some() { assert_eq!(result.unwrap(), case["entry"], "{}", case["name"]); }
            else { let error = result.unwrap_err(); assert_eq!(json!({"status":error.status,"code":error.code}), case["error"], "{}", case["name"]); }
        }
    }
    #[test] fn candidate_json_matches_existing_source_validation() {
        for case in cases()["suggestions"].as_array().unwrap() {
            let result = parse_suggestions(case["html"].as_str().unwrap(), case["language"].as_str().unwrap(), case["query"].as_str().unwrap());
            if case.get("words").is_some() { assert_eq!(json!(result.unwrap()), case["words"], "{}", case["name"]); }
            else { let error = result.unwrap_err(); assert_eq!(json!({"status":error.status,"code":error.code}), case["error"], "{}", case["name"]); }
        }
    }
    #[test] fn input_cannot_change_the_destination() {
        assert_eq!(dictionary_url("en-cn", "run on the spot").unwrap().as_str(), "https://dictionary.cambridge.org/us/dictionary/english-chinese-simplified/run%20on%20the%20spot");
        for word in ["", " ", "..", "/", "one\ntwo", "\\host", &"a".repeat(121)] { assert_eq!(dictionary_url("en-cn", word).unwrap_err().status, Some(400)); }
        assert_eq!(dictionary_url("constructor", "word").unwrap_err().status, Some(400));
        let url = dictionary_url("en", "https:example.com?x=y").unwrap();
        assert_eq!(url.host_str(), Some("dictionary.cambridge.org")); assert!(url.query().is_none());
        assert_eq!(suggestion_url("en-cn", "two words").unwrap().query_pairs().next().unwrap().1, "two words");
    }
}
