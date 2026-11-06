//! Parsing and code generation for the crate's `i18n!` macro.

use quote::quote;

use std::collections::BTreeMap;
use syn::parse;

/// Argument of the `i18n!` macro: the directory containing the locale files,
/// relative to `CARGO_MANIFEST_DIR`.
#[derive(Debug)]
pub(crate) struct Option {
    pub(crate) locales_path: String,
}

impl parse::Parse for Option {
    fn parse(input: parse::ParseStream) -> parse::Result<Self> {
        // The macro takes exactly one string literal; `syn::parse` rejects anything
        // left over after it.
        let locales_path = input.parse::<syn::LitStr>()?.value();

        Ok(Self { locales_path })
    }
}

/// Expand a flattened translation table into an `actix_cloud::map!` invocation.
///
/// `data` is keyed by `"{locale}.{key}"`, matching what `Locale::add_locale`
/// expects. The final `"" => ""` entry turns the trailing comma of the last real
/// entry into a separator and keeps the generated `map!` valid when there are no
/// translations at all.
pub(crate) fn generate_code(data: &BTreeMap<String, String>) -> proc_macro2::TokenStream {
    let mut locales = Vec::<proc_macro2::TokenStream>::new();

    for (k, v) in data {
        let k = k.to_owned();
        let v = v.to_owned();

        locales.push(quote! {
            #k => #v,
        });
    }

    // result
    quote! {
        actix_cloud::map! [
            #(#locales)*
            "" => ""    // eat last comma
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_locales_path() {
        let option: Option = syn::parse_str(r#""locales""#).unwrap();
        assert_eq!(option.locales_path, "locales");
    }

    #[test]
    fn parse_rejects_invalid_input() {
        // Unquoted path.
        assert!(syn::parse_str::<Option>("locales").is_err());
        // More than one token.
        assert!(syn::parse_str::<Option>(r#""locales" "extra""#).is_err());
    }

    #[test]
    fn generate_code_maps_flattened_keys() {
        let mut data = BTreeMap::new();
        data.insert("en-US.hello".to_owned(), "Hello".to_owned());
        data.insert("zh-CN.hello".to_owned(), "你好".to_owned());

        // `BTreeMap` iteration is sorted, so the output order is deterministic.
        let expected = quote! {
            actix_cloud::map! [
                "en-US.hello" => "Hello",
                "zh-CN.hello" => "你好",
                "" => ""
            ]
        };
        assert_eq!(generate_code(&data).to_string(), expected.to_string());
    }

    #[test]
    fn generate_code_handles_empty_translations() {
        let expected = quote! {
            actix_cloud::map! [
                "" => ""
            ]
        };
        assert_eq!(
            generate_code(&BTreeMap::new()).to_string(),
            expected.to_string()
        );
    }
}
