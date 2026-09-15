//! Small shared contract for the standalone locale DLL and its watcher.
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Machine-wide label data, shared by every user's Explorer and watcher.
pub fn catalogue_directory() -> Result<PathBuf, String> {
    let program_data = std::env::var_os("ProgramData").ok_or("ProgramData is unset")?;
    Ok(PathBuf::from(program_data).join("Divvun").join("Keyboards"))
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocaleOverride {
    pub abbreviation: Option<LocalizedText>,
    pub display_name: Option<LocalizedText>,
}

pub type LocaleConfig = BTreeMap<String, LocaleOverride>;

/// A fixed label (the original config format), or translations keyed by UI language.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum LocalizedText {
    Literal(String),
    Translations(BTreeMap<String, String>),
}

impl LocalizedText {
    /// Match UI preferences in order, trying each tag and its parents, then English.
    /// No matching translation means the caller should preserve Windows' result.
    pub fn resolve<'a>(&self, ui_languages: impl IntoIterator<Item = &'a str>) -> Option<&str> {
        match self {
            Self::Literal(text) => Some(text),
            Self::Translations(translations) => {
                let lookup = |tag: &str| {
                    translations
                        .iter()
                        .find(|(key, _)| key.eq_ignore_ascii_case(tag))
                        .map(|(_, text)| text.as_str())
                };
                for language in ui_languages {
                    let mut tag = language;
                    loop {
                        if let Some(text) = lookup(tag) {
                            return Some(text);
                        }
                        let Some((parent, _)) = tag.rsplit_once('-') else {
                            break;
                        };
                        tag = parent;
                    }
                }
                lookup("en")
            }
        }
    }
}

// Creation time prevents an old process's event being mistaken for a reused PID.
pub fn ready_event(pid: u32, creation_time: u64) -> String {
    format!("Local\\Divvun.Wind.KeyboardLabels.Ready.{pid}.{creation_time:016X}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translations_follow_ordered_ui_preferences_and_fallbacks() {
        let text: LocalizedText = serde_json::from_str(
            r#"{
            "en": "English", "ru": "Russian", "ru-RU": "Russian (Russia)"
        }"#,
        )
        .unwrap();
        assert_eq!(text.resolve(["RU-ru", "en-US"]), Some("Russian (Russia)"));
        assert_eq!(text.resolve(["ru-Cyrl-BY", "en"]), Some("Russian"));
        assert_eq!(text.resolve(["en-GB", "ru-RU"]), Some("English"));
        assert_eq!(text.resolve(["sv-SE", "ru-BY"]), Some("Russian"));
        assert_eq!(text.resolve(["sv-SE"]), Some("English"));
        assert_eq!(text.resolve([]), Some("English"));
        let only_ru: LocalizedText = serde_json::from_str(r#"{"ru":"Русский"}"#).unwrap();
        assert_eq!(only_ru.resolve(["en-US"]), None);
        let empty: LocalizedText = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.resolve(["ru-RU"]), None);
    }

    #[test]
    fn original_string_config_remains_compatible() {
        let config: LocaleConfig = serde_json::from_str(
            r#"{
            "sjd-cyrl": {"abbreviation":"КИЛ", "display_name":"Kildin Sami"}
        }"#,
        )
        .unwrap();
        let entry = &config["sjd-cyrl"];
        assert_eq!(
            entry.abbreviation.as_ref().unwrap().resolve(["ru-RU"]),
            Some("КИЛ")
        );
        assert_eq!(
            entry.display_name.as_ref().unwrap().resolve(["ru-RU"]),
            Some("Kildin Sami")
        );
    }
}
