use std::collections::HashMap;

pub use actix_cloud_codegen::i18n;

/// Get I18n text
///
/// ```no_run
/// use actix_cloud::{i18n::{i18n, Locale},t};
///
/// let mut locale = Locale::new("en-US").add_locale(i18n!("locale"));
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
/// # Examples
///
/// ```
/// use actix_cloud::map;
/// let val = map!{"key" => "value"};
/// ```
#[macro_export]
macro_rules! map {
    {$($key:expr => $value:expr),+} => {{
        let mut m = std::collections::HashMap::new();
        $(
            m.insert($key, $value);
        )+
        m
    }};
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug)]
pub struct Locale {
    pub locale: HashMap<String, String>,
    pub default: String,
}

impl Locale {
    pub fn new<S: Into<String>>(default: S) -> Self {
        Self {
            locale: HashMap::new(),
            default: default.into(),
        }
    }

    /// Add new locale items.
    pub fn add_locale<S: Into<String>>(mut self, l: HashMap<S, S>) -> Self {
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
