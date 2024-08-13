use std::collections::HashMap;

pub use actix_cloud_codegen::i18n;

/// Get I18n text
///
/// ```ignore
/// // Get default locale's text
/// t!(locale, "greeting");
///
/// // With variables
/// t!(locale, "messages.hello", name = "Jason");
///
/// // Get a special locale's text
/// t!(locale, "greeting", "de");
///
/// // With locale and variables
/// t!(locale, "messages.hello", "de", name = "Jason");
/// ```
#[macro_export]
macro_rules! t {
    ($l:ident, $key:expr) => {
        $l.translate(&$l.default, $key)
    };

    ($l:ident, $key:expr, $($var_name:tt = $var_val:expr),+) => {
        {
            let mut message = $l.translate(&$l.default, $key);
            $(
                message = message.replace(concat!("%{", stringify!($var_name), "}"), $var_val);
            )+
            message
        }
    };

    ($l:ident, $key:expr, $locale:expr) => {
        $l.translate($locale, $key)
    };

    ($l:ident, $key:expr, $locale:expr, $($var_name:tt = $var_val:expr),+) => {
        {
            let mut message = $l.translate($locale, $key);
            $(
                message = message.replace(concat!("%{", stringify!($var_name), "}"), $var_val);
            )+
            message
        }
    };
}

#[derive(Debug)]
pub struct Locale {
    pub locale: HashMap<String, String>,
    pub default: String,
}

impl Locale {
    pub fn new(default: String) -> Self {
        Self {
            locale: HashMap::new(),
            default,
        }
    }

    /// Add new locale items.
    pub fn add_locale<S: Into<String>>(&mut self, l: HashMap<S, S>) {
        self.locale
            .extend(l.into_iter().map(|(a, b)| (a.into(), b.into())));
    }

    /// Translate string.
    /// - Fallback to default language if not exist.
    /// - Again fallback to `key` if still not found.
    pub fn translate(&self, locale: &str, key: &str) -> String {
        let locale_key = format!("{locale}.{key}");
        self.locale.get(locale_key.as_str()).map_or_else(
            || {
                if locale == self.default {
                    key.to_owned()
                } else {
                    self.translate(&self.default, key)
                }
            },
            ToString::to_string,
        )
    }
}
