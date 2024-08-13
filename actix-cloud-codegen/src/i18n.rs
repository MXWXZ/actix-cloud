use quote::quote;

use std::collections::HashMap;
use syn::parse;

#[derive(Debug)]
pub(crate) struct Option {
    pub(crate) locales_path: String,
}

impl parse::Parse for Option {
    fn parse(input: parse::ParseStream) -> parse::Result<Self> {
        let locales_path = input.parse::<syn::LitStr>()?.value();

        Ok(Self { locales_path })
    }
}

pub(crate) fn generate_code(data: &HashMap<String, String>) -> proc_macro2::TokenStream {
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
