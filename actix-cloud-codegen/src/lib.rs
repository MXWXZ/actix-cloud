//! Procedural macros for [`actix-cloud`](https://crates.io/crates/actix-cloud).
//!
//! - [`main`]: attribute for the `main` function that runs it on the Actix Web runtime
//!   through `actix_cloud`'s re-exported `actix_web`.
//! - `i18n` (feature `i18n`): load locale files at compile time.

#![cfg_attr(docsrs, feature(doc_cfg))]

use proc_macro::TokenStream;
use quote::quote;
#[cfg(feature = "i18n")]
use std::{collections::BTreeMap, env, path};

#[cfg(feature = "i18n")]
mod i18n;

/// Attribute for the `main` function, equivalent to
/// `#[actix_web::rt::main(system = "actix_cloud::actix_web::rt::System")]`.
///
/// The runtime is taken from `actix_cloud`'s `actix_web` re-export, so the crate using
/// this macro does not need a direct `actix-web` dependency.
///
/// ```ignore
/// #[actix_cloud::main]
/// async fn main() -> std::io::Result<()> {
///     Ok(())
/// }
/// ```
#[proc_macro_attribute]
pub fn main(_: TokenStream, item: TokenStream) -> TokenStream {
    main_impl(item.into()).into()
}

/// Implementation of [`main`], kept on `proc_macro2` types so it can be unit tested
/// (`proc_macro::TokenStream` cannot be constructed outside a real macro expansion).
fn main_impl(item: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    quote! {
        #[actix_cloud::actix_web::rt::main(system = "actix_cloud::actix_web::rt::System")]
        #item
    }
}

#[cfg(feature = "i18n")]
/// Init I18n translations.
///
/// Loads every `**/*.yml` file under `locales_path` at compile time and expands to an
/// `actix_cloud::map!` table with flat `"locale.key"` entries, which is what
/// `actix_cloud::i18n::Locale::add_locale` expects.
///
/// The path is resolved relative to the package's `CARGO_MANIFEST_DIR`.
///
/// ```ignore
/// i18n!("locales");
/// ```
///
/// # Panics
///
/// Panics if `CARGO_MANIFEST_DIR` is not set or not valid unicode, or if the locale
/// files cannot be loaded.
#[proc_macro]
pub fn i18n(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let option = match syn::parse::<i18n::Option>(input) {
        Ok(input) => input,
        Err(err) => return err.to_compile_error().into(),
    };

    // CARGO_MANIFEST_DIR is current build directory
    let cargo_dir =
        env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set or not valid unicode");
    let locales_path = path::PathBuf::from(cargo_dir).join(option.locales_path);

    let translation = load_translations(&locales_path);
    let code = i18n::generate_code(&translation);

    if rust_i18n_support::is_debug() {
        println!("{code}");
    }

    code.into()
}

/// Load the locale files under `locales_path` and flatten them into `"locale.key"`
/// entries, the format used by the runtime translation table.
#[cfg(feature = "i18n")]
fn load_translations(locales_path: &path::Path) -> BTreeMap<String, String> {
    let data = rust_i18n_support::load_locales(&locales_path.display().to_string(), |_| false);
    let mut translation = BTreeMap::new();
    for (lang, mp) in data {
        for (k, v) in mp {
            translation.insert(format!("{lang}.{k}"), v);
        }
    }
    translation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_expands_to_actix_web_rt_attribute() {
        let item = quote! {
            async fn main() -> std::io::Result<()> {
                Ok(())
            }
        };
        let expected = quote! {
            #[actix_cloud::actix_web::rt::main(system = "actix_cloud::actix_web::rt::System")]
            async fn main() -> std::io::Result<()> {
                Ok(())
            }
        };

        assert_eq!(main_impl(item).to_string(), expected.to_string());
    }

    #[cfg(feature = "i18n")]
    #[test]
    fn loads_and_flattens_locales() {
        let path = path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/locales");
        let translations = load_translations(&path);

        assert_eq!(
            translations.get("en-US.hello").map(String::as_str),
            Some("Hello")
        );
        assert_eq!(
            translations.get("en-US.messages.greet").map(String::as_str),
            Some("Hello, %{name}!")
        );
        assert_eq!(
            translations.get("zh-CN.hello").map(String::as_str),
            Some("你好")
        );
    }
}
