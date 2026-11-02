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

/// What a failed capture tells the user, by the capture's stable code. 0.2.0 put its input
/// below the notice; here the input row is the card's top, hence "上方".
pub fn capture_notice(code: &str) -> Notice {
    let (title, message) = match code {
        "no_selection" => ("没有读到选中文字", "重新选中单词后再试，或在上方输入。"),
        "unreported_selection" => ("这里暂时无法取词", "重新选中后再试，或在上方输入。"),
        "unsupported" | "access_denied" => ("这里暂时无法取词", "可以在上方直接输入要查的词。"),
        "protected" => ("此处无法取词", "为保护隐私，不读取受保护的内容。"),
        "source_changed" => ("请重新选词", "回到原来的窗口，选中单词后再试。"),
        "invalid_selection" => ("请选中英文单词", "选择一个单词或简短短语，也可以直接输入。"),
        "copy_unavailable" => ("这里暂时无法取词", "重新选中后再试，或在上方直接输入。"),
        "clipboard_unavailable" => (
            "暂时无法保留剪贴板",
            "当前内容保持不变，可以在上方直接输入。",
        ),
        "clipboard_restore_failed" => ("剪贴板恢复未完成", "请检查剪贴板内容；本次没有继续查询。"),
        "timeout" => ("取词有些慢", "重新选词后再试，或在上方输入。"),
        _ => ("暂时没有读到文字", "重新选词后再试，或在上方输入。"),
    };
    notice(title, message, NoticeAction::Correct)
}

/// Turns a shortcut failure into advice without showing the raw error.
pub fn shortcut_notice(error: &str) -> &'static str {
    if error.contains("占用") {
        "这组快捷键已被占用，请换一组。"
    } else if error.contains("未响应") || error.contains("重新启动") {
        "快捷键暂不可用，请重启 WordSide。"
    } else if ["修饰键重复", "只能包含", "请使用 Ctrl", "需包含 Ctrl"]
        .iter()
        .any(|part| error.contains(part))
    {
        "请使用 Ctrl 或 Alt 搭配一个字母、数字或功能键。"
    } else {
        "快捷键未能保存，请稍后再试。"
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

    // Ported from 0.2.0's capture notice test.
    #[test]
    fn capture_failures_read_as_short_advice_without_internals() {
        for code in [
            "no_selection",
            "unreported_selection",
            "unsupported",
            "protected",
            "source_changed",
            "invalid_selection",
            "access_denied",
            "read_error",
            "copy_unavailable",
            "clipboard_unavailable",
            "clipboard_restore_failed",
            "timeout",
            "unknown",
            "__proto__",
            "constructor",
        ] {
            let value = capture_notice(code);
            assert!(value.message.chars().count() < 40, "{code}");
            for internal in ["API", "接口", "进程", "权限", "GetSelection"] {
                assert!(
                    !value.title.contains(internal) && !value.message.contains(internal),
                    "{code}"
                );
            }
        }
        assert_ne!(
            capture_notice("protected").message,
            capture_notice("no_selection").message
        );
        // Every code the capture layer reports has its own notice, not the fallback.
        use crate::capture::selection::Selection;
        for failure in [
            Selection::NoSelection,
            Selection::Protected,
            Selection::Timeout,
        ] {
            let (code, _) = failure.failure().unwrap();
            assert_ne!(capture_notice(code), capture_notice("read_error"), "{code}");
        }
    }

    // Ported from 0.2.0 `shortcut failures preserve useful conflict guidance without exposing raw errors`.
    #[test]
    fn shortcut_failures_keep_useful_guidance_without_raw_errors() {
        assert!(shortcut_notice("这组快捷键已被其他程序占用。原快捷键仍保留。").contains("占用"));
        assert!(shortcut_notice("快捷键服务未响应，请重新启动 WordSide。").contains("重启"));
        assert!(shortcut_notice("快捷键修饰键重复。").contains("Ctrl 或 Alt"));
        for raw in ["API HTTP 502 definition.translation UNKNOWN_OS_ERROR", ""] {
            assert_eq!(shortcut_notice(raw), "快捷键未能保存，请稍后再试。");
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
