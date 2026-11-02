//! What the card shows and how keys move through it. No drawing and no Windows calls here.
use crate::{
    entry::{identity, word_url, Definition, Entry, LookupError},
    presentation::{
        lookup_notice, pos_label, pronunciation_chips, Notice, NoticeAction, PronunciationChip,
        INVALID_INPUT,
    },
};

#[derive(Debug, Clone, PartialEq)]
pub struct PosGroup {
    pub key: String,
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Coverage {
    Bilingual,
    Partial,
    English,
    Empty,
}

/// Where one pronunciation chip stands. Only one sound loads or plays at a time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sound {
    Idle,
    Loading,
    Playing,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntryView {
    pub entry: Entry,
    pub groups: Vec<PosGroup>,
    pub chips: Vec<PronunciationChip>,
    pub sounds: Vec<Sound>,
    last_sound: Option<usize>,
    pub coverage: Coverage,
    pub tab: usize,
    pub selected: usize,
    pub expanded: Option<usize>,
}

impl EntryView {
    pub fn new(entry: Entry) -> Self {
        let mut groups: Vec<PosGroup> = Vec::new();
        for definition in &entry.definitions {
            let (label, key) = pos_label(&definition.pos);
            match groups.iter_mut().find(|group| group.key == key) {
                Some(group) => group.count += 1,
                None => groups.push(PosGroup {
                    key,
                    label,
                    count: 1,
                }),
            }
        }
        let translated = entry
            .definitions
            .iter()
            .filter(|d| !d.translation.trim().is_empty())
            .count();
        let coverage = match (entry.definitions.len(), translated) {
            (0, _) => Coverage::Empty,
            (total, done) if total == done => Coverage::Bilingual,
            (_, 0) => Coverage::English,
            _ => Coverage::Partial,
        };
        let chips = pronunciation_chips(&entry.pronunciation);
        Self {
            entry,
            groups,
            sounds: vec![Sound::Idle; chips.len()],
            last_sound: None,
            chips,
            coverage,
            tab: 0,
            selected: 0,
            expanded: Some(0),
        }
    }

    /// Definitions of the current part of speech. Numbers restart for each one.
    pub fn visible(&self) -> Vec<&Definition> {
        let Some(group) = self.groups.get(self.tab) else {
            return Vec::new();
        };
        self.entry
            .definitions
            .iter()
            .filter(|d| pos_label(&d.pos).1 == group.key)
            .collect()
    }

    pub fn origin(&self) -> Option<String> {
        (identity(&self.entry.query) != identity(&self.entry.word))
            .then(|| format!("{} → {}", self.entry.query, self.entry.word))
    }

    pub fn select_tab(&mut self, tab: usize) {
        if tab < self.groups.len() {
            self.tab = tab;
            self.selected = 0;
            self.expanded = Some(0);
        }
    }

    pub fn next_tab(&mut self) {
        if !self.groups.is_empty() {
            self.select_tab((self.tab + 1) % self.groups.len());
        }
    }

    pub fn move_selection(&mut self, delta: i32) {
        let count = self.visible().len() as i32;
        if count > 0 {
            self.selected = (self.selected as i32 + delta).clamp(0, count - 1) as usize;
        }
    }

    /// A click selects the row and opens its examples; a second click closes them.
    pub fn click_sense(&mut self, index: usize) {
        self.selected = index;
        self.expanded = if self.expanded == Some(index) {
            None
        } else {
            Some(index)
        };
    }

    pub fn set_expanded(&mut self, open: bool) {
        self.expanded = open.then_some(self.selected);
    }

    pub fn sound_url(&self, index: usize) -> Option<&str> {
        self.chips
            .get(index)
            .map(|chip| chip.url.as_str())
            .filter(|url| !url.is_empty())
    }

    pub fn has_sound(&self) -> bool {
        (0..self.chips.len()).any(|index| self.sound_url(index).is_some())
    }

    /// Ctrl+P: the next chip with a sound after the one played last, wrapping around.
    pub fn next_sound(&self) -> Option<usize> {
        let count = self.chips.len();
        let start = self.last_sound.map_or(0, |index| index + 1);
        (0..count)
            .map(|step| (start + step) % count)
            .find(|&index| self.sound_url(index).is_some())
    }

    /// A chip that starts loading stops whatever else was loading or playing.
    pub fn set_sound(&mut self, index: usize, sound: Sound) {
        if sound == Sound::Loading {
            self.stop_sounds();
            self.last_sound = Some(index);
        }
        if let Some(slot) = self.sounds.get_mut(index) {
            *slot = sound;
        }
    }

    pub fn stop_sounds(&mut self) {
        for sound in &mut self.sounds {
            if matches!(sound, Sound::Loading | Sound::Playing) {
                *sound = Sound::Idle;
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FailureView {
    pub notice: Notice,
    pub suggestions: Vec<String>,
    pub word: String,
    pub source_url: Option<String>,
}

impl FailureView {
    pub fn from_error(error: &LookupError, word: &str) -> Self {
        let notice = lookup_notice(error);
        let source_url = (notice.action == NoticeAction::Retry).then(|| word_url(word, "en-cn"));
        Self {
            notice,
            suggestions: error.suggestions.clone(),
            word: word.to_owned(),
            source_url,
        }
    }

    pub fn invalid_input() -> Self {
        Self {
            notice: INVALID_INPUT,
            suggestions: Vec::new(),
            word: String::new(),
            source_url: None,
        }
    }

    pub fn can_retry(&self) -> bool {
        self.notice.action == NoticeAction::Retry
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Empty,
    Loading,
    Entry(Box<EntryView>),
    Failure(FailureView),
}

/// What “复制这条释义” puts on the clipboard: the translation, then the English meaning,
/// one per line; whichever is missing is left out.
pub fn sense_text(definition: &Definition) -> String {
    [definition.translation.trim(), definition.text.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// The appearance choices, in the order the settings page shows them.
pub const THEMES: [(&str, &str); 3] = [("system", "跟随系统"), ("light", "浅色"), ("dark", "深色")];

pub fn theme_index(theme: &str) -> usize {
    THEMES.iter().position(|(id, _)| *id == theme).unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingsKey {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
}

/// What a key or click on the settings page asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingsChange {
    Nothing,
    Close,
    ToggleAuto,
    Record,
    StopRecording,
    Theme(&'static str),
}

/// The settings page: auto lookup, the lookup shortcut and the appearance, one per row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettingsView {
    pub row: usize,
    /// Waiting for the user to press a new shortcut.
    pub recording: bool,
    /// Why the last change didn't take, shown under the shortcut row.
    pub notice: Option<String>,
}

impl SettingsView {
    pub const ROWS: usize = 3;

    pub fn key(&mut self, key: SettingsKey, theme: &str) -> SettingsChange {
        if self.recording {
            return match key {
                SettingsKey::Escape => SettingsChange::StopRecording,
                _ => SettingsChange::Nothing,
            };
        }
        let current = theme_index(theme);
        match key {
            SettingsKey::Up => {
                self.row = self.row.saturating_sub(1);
                SettingsChange::Nothing
            }
            SettingsKey::Down => {
                self.row = (self.row + 1).min(Self::ROWS - 1);
                SettingsChange::Nothing
            }
            SettingsKey::Escape => SettingsChange::Close,
            SettingsKey::Enter => match self.row {
                0 => SettingsChange::ToggleAuto,
                1 => SettingsChange::Record,
                _ => SettingsChange::Theme(THEMES[(current + 1) % THEMES.len()].0),
            },
            SettingsKey::Left if self.row == 2 && current > 0 => {
                SettingsChange::Theme(THEMES[current - 1].0)
            }
            SettingsKey::Right if self.row == 2 && current + 1 < THEMES.len() => {
                SettingsChange::Theme(THEMES[current + 1].0)
            }
            SettingsKey::Left | SettingsKey::Right => SettingsChange::Nothing,
        }
    }
}

/// The shortcut text for a key pressed while recording, for `shortcut::parse` to judge.
/// None while only modifiers are down, so the user can keep building the combination.
pub fn pressed_combo(ctrl: bool, alt: bool, shift: bool, key: u16) -> Option<String> {
    let name = match key {
        // Shift, Ctrl, Alt and both Windows keys.
        0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5 => return None,
        0x30..=0x39 | 0x41..=0x5A => char::from(key as u8).to_string(),
        0x70..=0x87 => format!("F{}", key - 0x6F),
        other => format!("Key{other}"),
    };
    let mut parts = Vec::new();
    if ctrl {
        parts.push("Ctrl");
    }
    if alt {
        parts.push("Alt");
    }
    if shift {
        parts.push("Shift");
    }
    parts.push(&name);
    Some(parts.join("+"))
}

/// The only outside links the app opens: Cambridge pages over HTTPS.
pub fn allowed_link(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https" && url.host_str() == Some("dictionary.cambridge.org")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::read_api_entry;
    use serde_json::json;

    fn definition(pos: &str, translation: &str) -> serde_json::Value {
        json!({"pos":pos,"text":"meaning","translation":translation,"example":[]})
    }
    fn view(definitions: Vec<serde_json::Value>) -> EntryView {
        EntryView::new(read_api_entry(&json!({"entry":{"word":"run","pos":[],"definition":definitions,"pronunciation":[]}}), "run", "en-cn").unwrap())
    }

    #[test]
    fn groups_count_senses_per_part_of_speech_ignoring_grammar_codes() {
        let card = view(vec![
            definition("verb", "跑"),
            definition("noun [ C ]", "跑步"),
            definition("verb [ I ]", "经营"),
        ]);
        assert_eq!(
            card.groups
                .iter()
                .map(|g| (g.label.as_str(), g.count))
                .collect::<Vec<_>>(),
            [("动词", 2), ("名词", 1)]
        );
        assert_eq!(card.visible().len(), 2);
    }

    #[test]
    fn switching_tabs_shows_only_that_part_and_resets_the_selection() {
        let mut card = view(vec![
            definition("verb", "跑"),
            definition("verb", "经营"),
            definition("noun", "跑步"),
        ]);
        card.move_selection(1);
        card.next_tab();
        assert_eq!(card.visible()[0].translation, "跑步");
        assert_eq!((card.selected, card.expanded), (0, Some(0)));
        card.next_tab();
        assert_eq!(card.tab, 0, "tabs wrap around");
    }

    #[test]
    fn selection_stays_inside_the_list() {
        let mut card = view(vec![definition("verb", "跑"), definition("verb", "经营")]);
        card.move_selection(-1);
        assert_eq!(card.selected, 0);
        card.move_selection(5);
        assert_eq!(card.selected, 1);
    }

    #[test]
    fn clicking_a_sense_toggles_its_examples() {
        let mut card = view(vec![definition("verb", "跑"), definition("verb", "经营")]);
        card.click_sense(1);
        assert_eq!((card.selected, card.expanded), (1, Some(1)));
        card.click_sense(1);
        assert_eq!(card.expanded, None);
    }

    #[test]
    fn coverage_follows_available_translations() {
        assert_eq!(
            view(vec![definition("verb", "跑")]).coverage,
            Coverage::Bilingual
        );
        assert_eq!(
            view(vec![definition("verb", "跑"), definition("verb", " ")]).coverage,
            Coverage::Partial
        );
        assert_eq!(
            view(vec![definition("verb", "")]).coverage,
            Coverage::English
        );
        assert_eq!(view(vec![]).coverage, Coverage::Empty);
    }

    #[test]
    fn a_different_headword_is_shown_as_the_query_arrow() {
        let mut card = view(vec![]);
        assert_eq!(card.origin(), None);
        card.entry.query = "ran".into();
        assert_eq!(card.origin().unwrap(), "ran → run");
    }

    fn voiced(urls: &[&str]) -> EntryView {
        let pronunciation: Vec<_> = urls
            .iter()
            .enumerate()
            .map(|(index, url)| json!({"pos":"noun","lang":"uk","pron":format!("/p{index}/"),"url":url}))
            .collect();
        EntryView::new(read_api_entry(&json!({"entry":{"word":"run","pos":[],"definition":[],"pronunciation":pronunciation}}), "run", "en-cn").unwrap())
    }

    #[test]
    fn ctrl_p_cycles_through_chips_that_have_a_sound() {
        let base = "https://dictionary.cambridge.org/media/";
        let mut card = voiced(&[&format!("{base}a.mp3"), "", &format!("{base}c.mp3")]);
        assert!(card.has_sound());
        assert_eq!(card.next_sound(), Some(0));
        card.set_sound(0, Sound::Loading);
        assert_eq!(
            card.next_sound(),
            Some(2),
            "a chip without a sound is skipped"
        );
        card.set_sound(2, Sound::Loading);
        assert_eq!(card.next_sound(), Some(0));
        assert!(!voiced(&[""]).has_sound());
        assert_eq!(voiced(&[]).next_sound(), None);
    }

    #[test]
    fn only_one_sound_loads_or_plays_and_failures_stay_visible() {
        let base = "https://dictionary.cambridge.org/media/";
        let mut card = voiced(&[
            &format!("{base}a.mp3"),
            &format!("{base}b.mp3"),
            &format!("{base}c.mp3"),
        ]);
        card.set_sound(0, Sound::Failed);
        card.set_sound(1, Sound::Playing);
        card.set_sound(2, Sound::Loading);
        assert_eq!(card.sounds, [Sound::Failed, Sound::Idle, Sound::Loading]);
        card.stop_sounds();
        assert_eq!(card.sounds, [Sound::Failed, Sound::Idle, Sound::Idle]);
    }

    #[test]
    fn only_retryable_failures_offer_the_browser() {
        let blocked = FailureView::from_error(&LookupError::other("CAMBRIDGE_BLOCKED"), "term");
        assert!(blocked.can_retry());
        assert!(blocked.source_url.unwrap().ends_with("/term"));
        let missing = LookupError {
            not_found: true,
            suggestions: vec!["terminal".into()],
            code: "WORD_NOT_FOUND".into(),
            status: Some(404),
        };
        let failure = FailureView::from_error(&missing, "termnal");
        assert!(!failure.can_retry() && failure.source_url.is_none());
        assert_eq!(failure.suggestions, ["terminal"]);
    }

    #[test]
    fn a_copied_sense_is_the_translation_then_the_meaning() {
        let card = view(vec![
            definition("verb", " 跑 "),
            json!({"pos":"verb","text":"","translation":"经营","example":[]}),
            definition("verb", ""),
        ]);
        let senses = card.visible();
        assert_eq!(sense_text(senses[0]), "跑\r\nmeaning");
        assert_eq!(sense_text(senses[1]), "经营");
        assert_eq!(sense_text(senses[2]), "meaning");
    }

    #[test]
    fn the_settings_page_moves_by_row_and_each_row_has_its_change() {
        let mut page = SettingsView::default();
        assert_eq!(page.key(SettingsKey::Up, "system"), SettingsChange::Nothing);
        assert_eq!(page.row, 0);
        assert_eq!(
            page.key(SettingsKey::Enter, "system"),
            SettingsChange::ToggleAuto
        );
        page.key(SettingsKey::Down, "system");
        assert_eq!(
            page.key(SettingsKey::Enter, "system"),
            SettingsChange::Record
        );
        page.key(SettingsKey::Down, "system");
        page.key(SettingsKey::Down, "system");
        assert_eq!(page.row, 2, "the last row stays selected");
        assert_eq!(
            page.key(SettingsKey::Escape, "system"),
            SettingsChange::Close
        );
    }

    #[test]
    fn the_appearance_row_steps_through_the_choices() {
        let mut page = SettingsView {
            row: 2,
            ..Default::default()
        };
        assert_eq!(
            page.key(SettingsKey::Right, "system"),
            SettingsChange::Theme("light")
        );
        assert_eq!(
            page.key(SettingsKey::Left, "light"),
            SettingsChange::Theme("system")
        );
        assert_eq!(
            page.key(SettingsKey::Left, "system"),
            SettingsChange::Nothing
        );
        assert_eq!(
            page.key(SettingsKey::Right, "dark"),
            SettingsChange::Nothing
        );
        assert_eq!(
            page.key(SettingsKey::Enter, "dark"),
            SettingsChange::Theme("system"),
            "Enter wraps"
        );
        page.row = 0;
        assert_eq!(
            page.key(SettingsKey::Right, "system"),
            SettingsChange::Nothing
        );
    }

    #[test]
    fn while_recording_only_escape_does_anything() {
        let mut page = SettingsView {
            row: 1,
            recording: true,
            notice: None,
        };
        for key in [
            SettingsKey::Up,
            SettingsKey::Down,
            SettingsKey::Enter,
            SettingsKey::Left,
        ] {
            assert_eq!(page.key(key, "system"), SettingsChange::Nothing);
        }
        assert_eq!(page.row, 1);
        assert_eq!(
            page.key(SettingsKey::Escape, "system"),
            SettingsChange::StopRecording
        );
    }

    #[test]
    fn a_recorded_combination_reads_like_the_settings_file() {
        assert_eq!(
            pressed_combo(true, true, false, 0x41).unwrap(),
            "Ctrl+Alt+A"
        );
        assert_eq!(
            pressed_combo(false, true, true, 0x37).unwrap(),
            "Alt+Shift+7"
        );
        assert_eq!(pressed_combo(true, false, false, 0x7B).unwrap(), "Ctrl+F12");
        assert_eq!(
            pressed_combo(true, false, false, 0xBC).unwrap(),
            "Ctrl+Key188"
        );
        for modifier in [0x10, 0x11, 0x12, 0x5B, 0xA2] {
            assert_eq!(pressed_combo(true, false, false, modifier), None);
        }
        // What the parser then makes of them, as 0.2.0 does.
        assert_eq!(
            crate::shortcut::parse(&pressed_combo(true, true, false, 0x44).unwrap())
                .unwrap()
                .unwrap()
                .text,
            "Ctrl+Alt+D"
        );
        for refused in [
            pressed_combo(true, false, false, 0x7B),
            pressed_combo(false, false, true, 0x44),
            pressed_combo(true, false, false, 0xBC),
        ] {
            assert!(crate::shortcut::parse(&refused.unwrap()).is_err());
        }
    }

    #[test]
    fn only_cambridge_links_over_https_may_open() {
        assert!(allowed_link(
            "https://dictionary.cambridge.org/dictionary/english/run"
        ));
        for url in [
            "http://dictionary.cambridge.org/x",
            "https://example.com/",
            "https://dictionary.cambridge.org.evil.test/",
            "file:///c:/x",
            "nonsense",
        ] {
            assert!(!allowed_link(url), "{url}");
        }
    }
}
