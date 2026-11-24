//! Interface language, independent of the language requested from YouTube.
mod ru;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub enum AppLanguage {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "ru")]
    Russian,
}

impl AppLanguage {
    pub const OPTIONS: &[(Self, &str)] = &[(Self::English, "English"), (Self::Russian, "Русский")];

    fn translate(self, source: &str) -> &str {
        match self {
            Self::English => source,
            Self::Russian => ru::translate(source),
        }
    }
}

pub fn is_russian() -> bool {
    crate::store::settings_snapshot().app_language == AppLanguage::Russian
}

pub fn t(source: &str) -> &str {
    crate::store::settings_snapshot()
        .app_language
        .translate(source)
}

pub fn apply_component_locale() {
    gpui_kit::component::set_locale(if is_russian() { "ru" } else { "en" });
}

// Both messages are format strings so Rust checks their placeholders.
macro_rules! tf {
    ($en:literal, $ru:literal $(, $($args:tt)+)?) => {
        if $crate::i18n::is_russian() {
            format!($ru $(, $($args)+)?)
        } else {
            format!($en $(, $($args)+)?)
        }
    };
}

pub(crate) use tf;

#[cfg(test)]
mod tests {
    use super::AppLanguage;
    use crate::store::Settings;

    #[test]
    fn old_settings_keep_the_interface_in_english() {
        let settings: Settings = serde_json::from_str(r#"{"language":"ru"}"#).unwrap();
        assert_eq!(settings.app_language, AppLanguage::English);
        assert_eq!(settings.language, "ru");
    }

    #[test]
    fn russian_interface_round_trips_independently() {
        let settings: Settings =
            serde_json::from_str(r#"{"app_language":"ru","language":"en"}"#).unwrap();
        assert_eq!(settings.app_language, AppLanguage::Russian);
        assert_eq!(settings.language, "en");
        assert_eq!(
            serde_json::to_value(settings).unwrap()["app_language"],
            "ru"
        );
    }

    #[test]
    fn falls_back_to_the_source_text() {
        assert_eq!(AppLanguage::English.translate("Settings"), "Settings");
        assert_eq!(AppLanguage::Russian.translate("Settings"), "Настройки");
        assert_eq!(
            AppLanguage::Russian.translate("An untranslated label"),
            "An untranslated label"
        );
    }
}
