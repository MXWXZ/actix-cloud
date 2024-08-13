use proc_macro::TokenStream;
use quote::quote;
#[cfg(feature = "i18n")]
use rust_i18n_support::{is_debug, load_locales};
#[cfg(feature = "i18n")]
use std::{collections::HashMap, env, path};

#[cfg(feature = "i18n")]
mod i18n;

#[proc_macro_attribute]
pub fn main(_: TokenStream, item: TokenStream) -> TokenStream {
    let mut output: TokenStream = (quote! {
        #[::actix_cloud::actix_web::rt::main(system = "::actix_cloud::actix_web::rt::System")]
    })
    .into();

    output.extend(item);
    output
}

#[cfg(feature = "i18n")]
/// Init I18n translations.
///
/// This will load all translations by glob `**/*.yml` from the given path.
///
/// ```ignore
/// i18n!("locales");
/// ```
///
/// # Panics
///
/// Panics is variable `CARGO_MANIFEST_DIR` is empty.
#[proc_macro]
pub fn i18n(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let option = match syn::parse::<i18n::Option>(input) {
        Ok(input) => input,
        Err(err) => return err.to_compile_error().into(),
    };

    // CARGO_MANIFEST_DIR is current build directory
    let cargo_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is empty");
    let current_dir = path::PathBuf::from(cargo_dir);
    let locales_path = current_dir.join(option.locales_path);

    let data = load_locales(&locales_path.display().to_string(), |_| false);
    let mut translation = HashMap::new();
    for (lang, mp) in data {
        for (k, v) in mp {
            translation.insert(format!("{lang}.{k}"), v);
        }
    }
    let code = i18n::generate_code(&translation);

    if is_debug() {
        println!("{code}");
    }

    code.into()
}
