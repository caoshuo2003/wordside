//! Product copy chosen from stable states. Raw API and OS messages never reach the card.
//! Ported from `desktop/src/presentation.ts`.
use crate::entry::{LookupError, Pronunciation};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoticeAction {
    Correct,
    Retry,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub title: &'static str,
    pub message: &'static str,
    pub action: NoticeAction,
}

const fn notice(title: &'static str, message: &'static str, action: NoticeAction) -> Notice {
    Notice {
        title,
        message,
        action,
    }
}

pub const INVALID_INPUT: Notice = notice(
    "请换一个查询词",
    "输入一个英文单词或简短短语。",
    NoticeAction::Correct,
);

pub fn lookup_notice(error: &LookupError) -> Notice {
    use NoticeAction::*;
    if error.not_found {
        return if error.suggestions.is_empty() {
            notice("未找到这个词", "换一个拼写，或输入其他单词。", Correct)
        } else {
            notice("未找到这个词", "你可能想查：", Correct)
        };
    }
    match (error.code.as_str(), error.status) {
        ("INVALID_RESPONSE" | "INVALID_CAMBRIDGE_PAGE", _) => {
            notice("释义暂时不可用", "稍后再试，或查看原词典。", Retry)
        }
        ("CAMBRIDGE_TIMEOUT", _) | (_, Some(504)) => notice("查询有些慢", "稍后再试。", Retry),
        ("CAMBRIDGE_BLOCKED", _) => notice(
            "Cambridge 暂时拒绝了访问",
            "当前网络可能需要人机验证，可以在浏览器里打开这个词。",
            Retry,
        ),
        ("CAMBRIDGE_UNAVAILABLE", _) => {
            notice("暂时无法连接词典", "检查网络连接或代理后再试。", Retry)
        }
        ("INVALID_ENTRY" | "UNSUPPORTED_LANGUAGE", _) | (_, Some(400)) => INVALID_INPUT,
        _ => notice("查询暂未完成", "稍后再试。", Retry),
    }
}

/// Cambridge parts of speech; grammar codes such as "noun [ C ]" are dropped.
pub fn pos_label(pos: &str) -> (String, String) {
    let mut cleaned = String::new();
    let mut depth = 0;
    for c in pos.chars() {
        match c {
            '[' => depth += 1,
            ']' if depth > 0 => {
                depth -= 1;
                cleaned.push(' ');
            }
            _ if depth == 0 => cleaned.push(c),
            _ => {}
        }
    }
    let en = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let zh = match en.as_str() {
        "noun" => "名词",
        "verb" => "动词",
        "adjective" => "形容词",
        "adverb" => "副词",
        "preposition" => "介词",
        "conjunction" => "连词",
        "pronoun" => "代词",
        "determiner" => "限定词",
        "predeterminer" => "前位限定词",
        "exclamation" | "interjection" => "感叹词",
        "number" => "数词",
        "ordinal number" => "序数词",
        "prefix" => "前缀",
        "suffix" => "后缀",
        "auxiliary verb" => "助动词",
        "modal verb" => "情态动词",
        "phrasal verb" => "短语动词",
        "idiom" => "习语",
        "phrase" => "短语",
        "abbreviation" => "缩写",
        "combining form" => "构词成分",
        "plural noun" => "复数名词",
        "symbol" => "符号",
        "article" => "冠词",
        other => other,
    };
    (zh.to_owned(), en)
}

/// Cambridge marks senses from its other dictionaries with a dataset id.
pub fn source_label(id: &str) -> String {
    match id.to_lowercase().as_str() {
        "cald4" => "剑桥高阶英语".into(),
        "cacd" => "剑桥美式英语".into(),
        "cbed" => "剑桥商务英语".into(),
        _ => id.to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PronunciationChip {
    pub lang: String,
    pub pron: String,
    pub url: String,
    pub pos: String,
}

/// One chip per distinct sound. Identical readings under several parts of speech collapse;
/// differing readings keep their part of speech so none is lost.
pub fn pronunciation_chips(list: &[Pronunciation]) -> Vec<PronunciationChip> {
    let mut chips: Vec<PronunciationChip> = Vec::new();
    let mut keys: Vec<(String, String)> = Vec::new();
    let mut parts: Vec<Vec<String>> = Vec::new();
    for item in list {
        let pron = item.pron.trim();
        if pron.is_empty() {
            continue;
        }
        let key = (item.lang.to_lowercase(), pron.to_owned());
        let index = match keys.iter().position(|existing| *existing == key) {
            Some(index) => {
                if chips[index].url.is_empty() && !item.url.is_empty() {
                    chips[index].url = item.url.clone();
                }
                index
            }
            None => {
                let lang = match key.0.as_str() {
                    "uk" => "英".to_owned(),
                    "us" => "美".to_owned(),
                    _ => item.lang.to_uppercase(),
                };
                chips.push(PronunciationChip {
                    lang,
                    pron: pron.to_owned(),
                    url: item.url.clone(),
                    pos: String::new(),
                });
                keys.push(key);
                parts.push(Vec::new());
                chips.len() - 1
            }
        };
        let pos = pos_label(&item.pos).0;
        if !pos.is_empty() && !parts[index].contains(&pos) {
            parts[index].push(pos);
        }
    }
    let languages: Vec<String> = chips.iter().map(|chip| chip.lang.clone()).collect();
    for (index, chip) in chips.iter_mut().enumerate() {
        if languages.iter().filter(|lang| **lang == chip.lang).count() > 1 {
            chip.pos = parts[index].join("/");
        }
    }
    chips
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::read_api_entry;
    use serde_json::json;

    const RAW: &str = "API HTTP 502 definition.translation UNKNOWN_OS_ERROR";

    fn notice_for(payload: serde_json::Value) -> Notice {
        lookup_notice(&read_api_entry(&payload, "termnal", "en-cn").unwrap_err())
    }

    #[test]
    fn missing_words_keep_correction_separate_from_connectivity_or_invalid_content() {
        let missing = notice_for(
            json!({"entry":null,"apiError":RAW,"apiStatus":404,"apiCode":"WORD_NOT_FOUND","suggestions":["terminal"]}),
        );
        assert_eq!(missing.title, "未找到这个词");
        assert_eq!(missing.action, NoticeAction::Correct);
        let mut titles = Vec::new();
        for (code, status) in [
            ("CAMBRIDGE_UNAVAILABLE", 502),
            ("CAMBRIDGE_BLOCKED", 502),
            ("CAMBRIDGE_TIMEOUT", 504),
            ("INVALID_CAMBRIDGE_PAGE", 502),
            ("OTHER", 502),
        ] {
            let value = notice_for(
                json!({"entry":null,"apiError":RAW,"apiStatus":status,"apiCode":code,"suggestions":["terminal"]}),
            );
            assert_eq!(value.action, NoticeAction::Retry);
            assert_ne!(value.title, missing.title);
            let shown = format!("{} {}", value.title, value.message);
            assert!(
                ["API", "HTTP", "definition", "502", "504"]
                    .iter()
                    .all(|part| !shown.contains(part)),
                "{shown}"
            );
            titles.push(value.title);
        }
        titles.sort();
        titles.dedup();
        assert_eq!(titles.len(), 5, "each failure reads differently");
    }

    #[test]
    fn malformed_json_and_candidate_data_never_leak_field_diagnostics() {
        for payload in [
            json!({"entry":{}}),
            json!({"entry":{"word":"terminal","pos":[],"definition":"bad","pronunciation":[]}}),
            json!({"entry":null,"apiError":RAW,"apiStatus":404,"apiCode":"WORD_NOT_FOUND","suggestions":["bad/path"]}),
        ] {
            let value = notice_for(payload);
            assert_eq!(value.title, "释义暂时不可用");
            assert_eq!(value.action, NoticeAction::Retry);
        }
    }

    #[test]
    fn unknown_failures_become_ordinary_product_copy() {
        let value = lookup_notice(&LookupError::other(RAW));
        assert_eq!(value.title, "查询暂未完成");
        assert_eq!(value.action, NoticeAction::Retry);
    }

    #[test]
    fn invalid_input_guides_correction() {
        assert_eq!(INVALID_INPUT.action, NoticeAction::Correct);
        assert_eq!(INVALID_INPUT.title, "请换一个查询词");
    }

    #[test]
    fn parts_of_speech_read_in_chinese_without_grammar_codes() {
        assert_eq!(pos_label("noun [ C ]"), ("名词".into(), "noun".into()));
        assert_eq!(
            pos_label("Phrasal verb"),
            ("短语动词".into(), "phrasal verb".into())
        );
        assert_eq!(pos_label("suffix"), ("后缀".into(), "suffix".into()));
        assert_eq!(
            pos_label("combining form"),
            ("构词成分".into(), "combining form".into())
        );
        assert_eq!(
            pos_label("toString"),
            ("tostring".into(), "tostring".into())
        );
    }

    #[test]
    fn dataset_ids_read_as_dictionary_names() {
        assert_eq!(source_label("cbed"), "剑桥商务英语");
        assert_eq!(source_label("CALD4"), "剑桥高阶英语");
        assert_eq!(source_label("xyz"), "xyz");
        assert_eq!(source_label("constructor"), "constructor");
    }

    fn pron(pos: &str, lang: &str, pron: &str, url: &str) -> Pronunciation {
        Pronunciation {
            pos: pos.into(),
            lang: lang.into(),
            pron: pron.into(),
            url: url.into(),
        }
    }

    #[test]
    fn identical_readings_collapse_and_differing_readings_keep_every_sound() {
        let run = pronunciation_chips(&[
            pron("verb", "uk", "/rʌn/", "uk"),
            pron("verb", "us", "/rʌn/", "us"),
            pron("noun", "uk", "/rʌn/", ""),
            pron("noun", "us", "/rʌn/", "us2"),
        ]);
        assert_eq!(
            run.iter()
                .map(|c| (c.lang.as_str(), c.url.as_str(), c.pos.as_str()))
                .collect::<Vec<_>>(),
            [("英", "uk", ""), ("美", "us", "")]
        );
        let record = pronunciation_chips(&[
            pron("noun", "uk", "/ˈrek.ɔːd/", ""),
            pron("adjective", "uk", "/ˈrek.ɔːd/", "a"),
            pron("verb", "uk", "/rɪˈkɔːd/", "v"),
            pron("noun", "us", "/ˈrek.ɚd/", ""),
        ]);
        assert_eq!(
            record
                .iter()
                .map(|c| (c.lang.as_str(), c.url.as_str(), c.pos.as_str()))
                .collect::<Vec<_>>(),
            [
                ("英", "a", "名词/形容词"),
                ("英", "v", "动词"),
                ("美", "", "")
            ]
        );
        assert!(pronunciation_chips(&[pron("noun", "uk", "  ", "")]).is_empty());
    }
}
