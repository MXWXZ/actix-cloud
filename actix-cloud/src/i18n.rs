use std::collections::BTreeMap;

pub use actix_cloud_codegen::i18n;

/// Get I18n text.
///
/// Without a locale argument the default locale of the [`Locale`] is used. Variables
/// are substituted for `%{name}` placeholders in the translated text; the same name
/// may be supplied for either the default locale or an explicit locale.
///
/// ```no_run
/// use actix_cloud::{i18n::{i18n, Locale},t};
///
/// let locale = Locale::new("en-US").add_locale(i18n!("locale"));
///
/// // Get default locale's text
/// t!(locale, "greeting");
/// // With variables
/// t!(locale, "messages.hello", name = "Jason");
/// // Get a special locale's text
/// t!(locale, "greeting", "de");
/// // With locale and variables
/// t!(locale, "messages.hello", "de", name = "Jason");
/// ```
#[macro_export]
macro_rules! t {
    ($l:expr, $key:expr) => {
        $l.translate(&$l.default, $key)
    };

    ($l:expr, $key:expr, $($var_name:tt = $var_val:expr),+) => {
        {
            let mut message = $l.translate(&$l.default, $key);
            $(
                message = message.replace(concat!("%{", stringify!($var_name), "}"), $var_val);
            )+
            message
        }
    };

    ($l:expr, $key:expr, $locale:expr) => {
        $l.translate($locale, $key)
    };

    ($l:expr, $key:expr, $locale:expr, $($var_name:tt = $var_val:expr),+) => {
        {
            let mut message = $l.translate($locale, $key);
            $(
                message = message.replace(concat!("%{", stringify!($var_name), "}"), $var_val);
            )+
            message
        }
    };
}

/// Make map creation easier.
///
/// Builds a [`std::collections::BTreeMap`]. At least one key-value pair is required.
///
/// # Examples
///
/// ```
/// use actix_cloud::map;
/// let val = map!{"key" => "value"};
/// assert_eq!(val.get("key"), Some(&"value"));
/// ```
#[macro_export]
macro_rules! map {
    {$($key:expr => $value:expr),+} => {{
        let mut m = std::collections::BTreeMap::new();
        $(
            m.insert($key, $value);
        )+
        m
    }};
}

/// Translation table plus the default locale used as fallback.
///
/// Load translations at compile time with `Locale::new("en-US").add_locale(i18n!("locale"))`
/// and translate with the [`t!`] macro.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug)]
pub struct Locale {
    /// Flattened map of `"locale.key"` to translated text.
    pub locale: BTreeMap<String, String>,
    /// Default locale identifier, e.g. `en-US`.
    pub default: String,
}

impl Locale {
    /// Create a locale table with `default` as the fallback locale.
    pub fn new<S: Into<String>>(default: S) -> Self {
        Self {
            locale: BTreeMap::new(),
            default: default.into(),
        }
    }

    /// Add new locale items.
    pub fn add_locale<S: Into<String>>(mut self, l: BTreeMap<S, S>) -> Self {
        self.locale
            .extend(l.into_iter().map(|(a, b)| (a.into(), b.into())));
        self
    }

    /// Translate string.
    /// - Fallback to default language if not exist.
    /// - Again fallback to `key` if still not found.
    pub fn translate<S1: AsRef<str>, S2: AsRef<str>>(&self, locale: S1, key: S2) -> String {
        let locale_key = format!("{}.{}", locale.as_ref(), key.as_ref());
        self.locale.get(locale_key.as_str()).map_or_else(
            || {
                if locale.as_ref() == self.default {
                    key.as_ref().to_owned()
                } else {
                    self.translate(&self.default, key)
                }
            },
            ToString::to_string,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locale() -> Locale {
        Locale::new("en-US").add_locale(crate::map! {
            "en-US.greeting" => "Hello",
            "en-US.welcome" => "Hello, %{name}!",
            "zh-CN.greeting" => "你好",
            "zh-CN.only_zh" => "只有中文"
        })
    }

    #[test]
    fn translate_falls_back_to_default_then_key() {
        let locale = locale();

        assert_eq!(locale.translate("en-US", "greeting"), "Hello");
        assert_eq!(locale.translate("zh-CN", "greeting"), "你好");

        // Unknown locale falls back to the default locale.
        assert_eq!(locale.translate("de-DE", "greeting"), "Hello");
        // Known locale missing the key also falls back to the default locale.
        assert_eq!(locale.translate("zh-CN", "welcome"), "Hello, %{name}!");
        // Missing in the default locale falls back to the key itself.
        assert_eq!(locale.translate("en-US", "missing"), "missing");
    }

    #[test]
    fn t_macro_uses_default_or_explicit_locale() {
        let locale = locale();

        assert_eq!(crate::t!(locale, "greeting"), "Hello");
        assert_eq!(crate::t!(locale, "greeting", "zh-CN"), "你好");
        assert_eq!(crate::t!(locale, "missing"), "missing");
    }

    #[test]
    fn t_macro_substitutes_variables() {
        let locale = locale();

        assert_eq!(
            crate::t!(locale, "welcome", name = "Jason"),
            "Hello, Jason!"
        );
        assert_eq!(
            crate::t!(locale, "welcome", "en-US", name = "Jason"),
            "Hello, Jason!"
        );
        // Unknown placeholders are left untouched.
        assert_eq!(crate::t!(locale, "welcome", other = "x"), "Hello, %{name}!");
    }

    #[test]
    fn map_macro_builds_sorted_map() {
        let m = crate::map! {"b" => 2, "a" => 1};
        assert_eq!(m.keys().copied().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(m["a"], 1);
    }

    #[test]
    fn add_locale_overwrites_existing_entries() {
        let locale = Locale::new("en-US")
            .add_locale(crate::map! {"en-US.key" => "first"})
            .add_locale(crate::map! {"en-US.key" => "second"});

        assert_eq!(locale.translate("en-US", "key"), "second");
    }

    #[test]
    fn t_macro_replaces_every_occurrence_literally() {
        let locale = Locale::new("en-US").add_locale(crate::map! {
            "en-US.eq" => "%{a} + %{a} = %{b}"
        });

        // Replacement is literal: `$`, `%` and braces in values are not special.
        assert_eq!(
            crate::t!(locale, "eq", a = "$1%{}", b = "2"),
            "$1%{} + $1%{} = 2"
        );
        // All variables must be supplied by the caller; extra ones are ignored.
        assert_eq!(
            crate::t!(locale, "eq", a = "x", b = "y", unused = "z"),
            "x + x = y"
        );
    }

    #[test]
    fn empty_default_locale_falls_back_to_the_key() {
        let locale = Locale::new("");

        assert_eq!(locale.translate("", "missing"), "missing");
        assert_eq!(locale.translate("zh-CN", "missing"), "missing");
    }

    #[test]
    fn map_macro_keeps_the_last_duplicate() {
        let m = crate::map! {"a" => 1, "a" => 2};
        assert_eq!(m["a"], 2);
        assert_eq!(m.len(), 1);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn locale_roundtrips_through_serde() {
        let locale = locale();
        let json = serde_json::to_string(&locale).unwrap();
        let decoded: Locale = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.default, "en-US");
        assert_eq!(decoded.translate("zh-CN", "greeting"), "你好");
    }
}
