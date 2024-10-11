#![cfg_attr(docsrs, feature(doc_auto_cfg))]

use proc_macro::TokenStream;
use quote::quote;
#[cfg(feature = "i18n")]
use std::{collections::HashMap, env, path};

#[cfg(feature = "i18n")]
mod i18n;

#[proc_macro_attribute]
pub fn main(_: TokenStream, item: TokenStream) -> TokenStream {
    let mut output: TokenStream = (quote! {
        #[actix_cloud::actix_web::rt::main(system = "actix_cloud::actix_web::rt::System")]
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

    let data = rust_i18n_support::load_locales(&locales_path.display().to_string(), |_| false);
    let mut translation = HashMap::new();
    for (lang, mp) in data {
        for (k, v) in mp {
            translation.insert(format!("{lang}.{k}"), v);
        }
    }
    let code = i18n::generate_code(&translation);

    if rust_i18n_support::is_debug() {
        println!("{code}");
    }

    code.into()
}

#[cfg(feature = "seaorm")]
/// Default timestamp generator.
///
/// Automatically generate `created_at` and `updated_at` on create and update.
///
/// # Examples
/// ```ignore
/// pub struct Model {
///     ...
///     pub created_at: i64,
///     pub updated_at: i64,
/// }
///
/// #[entity_timestamp]
/// impl ActiveModel {}
/// ```
#[proc_macro_attribute]
pub fn entity_timestamp(_: TokenStream, input: TokenStream) -> TokenStream {
    let mut entity = syn::parse_macro_input!(input as syn::ItemImpl);
    entity.items.push(syn::parse_quote!(
        fn entity_timestamp(&self, e: &mut Self, insert: bool) {
            let tm: sea_orm::ActiveValue<i64> =
                sea_orm::ActiveValue::set(chrono::Utc::now().timestamp_millis());
            if insert {
                e.created_at = tm.clone();
                e.updated_at = tm.clone();
            } else {
                e.updated_at = tm.clone();
            }
        }
    ));
    quote! {
        #entity
    }
    .into()
}

#[cfg(feature = "seaorm")]
/// Default id generator.
///
/// Automatically generate `id` on create.
///
/// # Examples
/// ```ignore
/// pub struct Model {
///     id: i64,
///     ...
/// }
///
/// #[entity_id(rand_i64())]
/// impl ActiveModel {}
/// ```
#[proc_macro_attribute]
pub fn entity_id(attr: TokenStream, input: TokenStream) -> TokenStream {
    let attr = syn::parse_macro_input!(attr as syn::ExprCall);
    let mut entity = syn::parse_macro_input!(input as syn::ItemImpl);
    entity.items.push(syn::parse_quote!(
        fn entity_id(&self, e: &mut Self, insert: bool) {
            if insert && e.id.is_not_set() {
                e.id = sea_orm::ActiveValue::set(#attr);
            }
        }
    ));
    quote! {
        #entity
    }
    .into()
}

#[cfg(feature = "seaorm")]
/// Default entity behavior:
/// - `entity_id`
/// - `entity_timestamp`
///
/// # Examples
/// ```ignore
/// #[entity_id(rand_i64())]
/// #[entity_timestamp]
/// impl ActiveModel {}
///
/// #[entity_behavior]
/// impl ActiveModelBehavior for ActiveModel {}
/// ```
#[proc_macro_attribute]
pub fn entity_behavior(_: TokenStream, input: TokenStream) -> TokenStream {
    let mut entity = syn::parse_macro_input!(input as syn::ItemImpl);

    entity.items.push(syn::parse_quote!(
        async fn before_save<C>(self, _: &C, insert: bool) -> Result<Self, DbErr>
        where
            C: ConnectionTrait,
        {
            let mut new = self.clone();
            self.entity_id(&mut new, insert);
            self.entity_timestamp(&mut new, insert);
            Ok(new)
        }
    ));
    quote! {
        #[async_trait::async_trait]
        #entity
    }
    .into()
}

#[cfg(feature = "seaorm")]
/// Implement `into` for entity to partial entity.
/// The fields should be exactly the same.
///
/// # Examples
/// ```ignore
/// #[partial_entity(users::Model)]
/// #[derive(Serialize)]
/// struct Rsp {
///     pub id: i64,
/// }
///
/// let y = users::Model {
///     id: ...,
///     name: ...,
///     ...
/// };
/// let x: Rsp = y.into();
/// ```
#[proc_macro_attribute]
pub fn partial_entity(attr: TokenStream, input: TokenStream) -> TokenStream {
    let attr = syn::parse_macro_input!(attr as syn::ExprPath);
    let input = syn::parse_macro_input!(input as syn::ItemStruct);
    let name = &input.ident;
    let mut fields = Vec::new();
    for i in &input.fields {
        let field_name = &i.ident;
        fields.push(quote!(#field_name: self.#field_name,));
    }

    quote! {
        #input
        impl Into<#name> for #attr {
            fn into(self) -> #name {
                #name {
                    #(#fields)*
                }
            }
        }
    }
    .into()
}
