//! Preferences in `%APPDATA%\local.wordside.dictionary.api-preview\settings.json`, the file
//! 0.2.0 writes. Each field is read on its own, so one bad value never resets the others, and
//! fields this version doesn't use (palette, popup_size, close_behavior, ...) are written back
//! untouched so going back to 0.2.0 loses nothing.
use serde_json::{json, Map, Value};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

/// Changing it would lose every user's settings; it is the folder 0.2.0 used.
const IDENTIFIER: &str = "local.wordside.dictionary.api-preview";
pub const DEFAULT_HOTKEY: &str = "Ctrl+Alt+A";
const THEMES: [&str; 3] = ["system", "light", "dark"];
const KNOWN: [&str; 3] = ["enabled", "theme", "lookup_hotkey"];

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Automatic lookup on selection. Off until the user turns it on.
    pub enabled: bool,
    /// `system`, `light` or `dark`.
    pub theme: String,
    /// Empty means the shortcut is turned off.
    pub lookup_hotkey: String,
    /// Everything else in the file, kept as it was.
    other: Map<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            theme: "system".into(),
            lookup_hotkey: DEFAULT_HOTKEY.into(),
            other: Map::new(),
        }
    }
}

impl Settings {
    pub fn decode(bytes: &[u8]) -> Self {
        let defaults = Self::default();
        let Ok(Value::Object(mut fields)) = serde_json::from_slice::<Value>(bytes) else {
            return defaults;
        };
        let enabled = fields
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(defaults.enabled);
        let theme = fields
            .get("theme")
            .and_then(Value::as_str)
            .filter(|theme| THEMES.contains(theme))
            .map_or(defaults.theme, str::to_owned);
        let lookup_hotkey = fields
            .get("lookup_hotkey")
            .and_then(Value::as_str)
            .map_or(defaults.lookup_hotkey, str::to_owned);
        for key in KNOWN {
            fields.remove(key);
        }
        Self {
            enabled,
            theme,
            lookup_hotkey,
            other: fields,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut fields = self.other.clone();
        fields.insert("enabled".into(), json!(self.enabled));
        fields.insert("theme".into(), json!(self.theme));
        fields.insert("lookup_hotkey".into(), json!(self.lookup_hotkey));
        serde_json::to_vec_pretty(&Value::Object(fields)).unwrap_or_default()
    }
}

pub fn path() -> Option<PathBuf> {
    let roaming = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(roaming)
            .join(IDENTIFIER)
            .join("settings.json"),
    )
}

/// A missing or unreadable file gives the defaults.
pub fn load(path: &Path) -> Settings {
    fs::read(path)
        .map(|bytes| Settings::decode(&bytes))
        .unwrap_or_default()
}

/// Writes a temporary file beside the target and renames it over, so a crash mid-write
/// leaves the previous settings intact.
// The settings page, later in this milestone, is the first caller.
#[allow(dead_code)]
pub fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    let folder = path
        .parent()
        .ok_or_else(|| io::Error::other("settings folder unavailable"))?;
    fs::create_dir_all(folder)?;
    let temporary = folder.join(format!(".settings-{}.tmp", std::process::id()));
    let written = (|| {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&settings.encode())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("wordside-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        folder
    }

    // Ported from 0.2.0 `close_preference_migrates_without_resetting_other_settings`.
    #[test]
    fn odd_values_in_other_fields_never_reset_the_preferences() {
        for value in [
            json!(null),
            json!("unknown"),
            json!(123),
            json!({"obsolete":true}),
        ] {
            let data = json!({"enabled":true,"theme":"dark","palette":"blue","lookup_hotkey":"Alt+D","popup_size":{"width":520,"height":640},"close_behavior":value});
            let settings = Settings::decode(&serde_json::to_vec(&data).unwrap());
            assert!(settings.enabled);
            assert_eq!(settings.theme, "dark");
            assert_eq!(settings.lookup_hotkey, "Alt+D");
            assert_eq!(settings.other["close_behavior"], value, "kept for 0.2.0");
            assert_eq!(settings.other["popup_size"]["width"], 520);
        }
        let short = Settings::decode(br#"{"enabled":true,"lookup_hotkey":"Alt+D"}"#);
        assert!(short.enabled);
        assert_eq!(short.theme, "system");
    }

    // Ported from 0.2.0 `close_preference_is_atomic_and_survives_file_reload`.
    #[test]
    fn saving_is_atomic_and_survives_a_reload() {
        let folder = folder("atomic");
        let config = folder.join("settings.json");
        let mut settings = Settings::decode(br#"{"enabled":true,"theme":"dark","palette":"lavender","lookup_hotkey":"Alt+D","popup_size":{"width":520,"height":640},"close_behavior":"exit"}"#);
        for theme in ["light", "system"] {
            settings.theme = theme.into();
            save(&config, &settings).unwrap();
            let read = load(&config);
            assert_eq!(read, settings);
            assert_eq!(read.other["palette"], "lavender");
            assert_eq!(read.other["close_behavior"], "exit");
            assert_eq!(read.other["popup_size"]["height"], 640);
        }
        assert_eq!(
            fs::read_dir(&folder).unwrap().count(),
            1,
            "temporary settings files are cleaned up"
        );
        fs::remove_dir_all(folder).unwrap();
    }

    // Ported from 0.2.0 `popup_size_survives_restart_without_resetting_old_preferences`.
    #[test]
    fn popup_size_survives_a_restart_without_resetting_old_preferences() {
        let legacy =
            Settings::decode(br#"{"enabled":true,"theme":"dark","lookup_hotkey":"Alt+D"}"#);
        assert!(!legacy.other.contains_key("popup_size"));
        let settings = Settings::decode(br#"{"enabled":true,"theme":"dark","palette":"blue","lookup_hotkey":"Alt+D","popup_size":{"width":520,"height":640}}"#);
        let again = Settings::decode(&settings.encode());
        assert_eq!(again.other["popup_size"], json!({"width":520,"height":640}));
        assert!(again.enabled);
        assert_eq!(again.lookup_hotkey, "Alt+D");
        assert_eq!(again.other["palette"], "blue");
        for size in [
            json!(null),
            json!({"width":"bad","height":-1}),
            json!({"width":999999,"height":0}),
        ] {
            let data =
                json!({"enabled":true,"theme":"dark","lookup_hotkey":"Alt+D","popup_size":size});
            let read = Settings::decode(&serde_json::to_vec(&data).unwrap());
            assert!(read.enabled);
            assert_eq!(read.lookup_hotkey, "Alt+D");
            assert_eq!(
                read.other["popup_size"], size,
                "0.2.0 still validates it itself"
            );
        }
    }

    #[test]
    fn a_wrongly_typed_field_falls_back_alone() {
        let settings = Settings::decode(
            br#"{"enabled":"yes","theme":"purple","lookup_hotkey":7,"palette":"stone"}"#,
        );
        assert_eq!(
            (
                settings.enabled,
                settings.theme.as_str(),
                settings.lookup_hotkey.as_str()
            ),
            (false, "system", DEFAULT_HOTKEY)
        );
        assert_eq!(settings.other["palette"], "stone");
        for broken in [&b""[..], b"not json", b"[1,2]", b"null"] {
            assert_eq!(Settings::decode(broken), Settings::default());
        }
    }

    #[test]
    fn a_turned_off_shortcut_stays_off() {
        let settings = Settings::decode(br#"{"lookup_hotkey":""}"#);
        assert_eq!(settings.lookup_hotkey, "");
        assert_eq!(Settings::decode(&settings.encode()).lookup_hotkey, "");
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        assert_eq!(
            load(&folder("missing").join("settings.json")),
            Settings::default()
        );
    }

    #[test]
    fn settings_live_where_0_2_0_kept_them() {
        let path = path().unwrap();
        assert!(path.ends_with(r"local.wordside.dictionary.api-preview\settings.json"));
    }
}
