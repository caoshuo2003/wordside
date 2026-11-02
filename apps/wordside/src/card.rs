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
