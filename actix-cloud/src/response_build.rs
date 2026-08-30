//! Build-time code generation for response codes (feature `response-build`).
//!
//! Call [`generate_response`] from `build.rs` to walk a directory of YAML files and
//! emit one enum per file implementing [`ResponseCodeTrait`](crate::response::ResponseCodeTrait).
//! See the `examples/response` example for the full setup.
use anyhow::Result;
use quote::{format_ident, quote};
use std::{
    env,
    fs::{read_to_string, File},
    io::Write,
    path::Path,
};
use walkdir::WalkDir;
use yaml_rust2::YamlLoader;

/// Errors raised while parsing response YAML files.
#[derive(thiserror::Error, Debug)]
pub enum BuildError {
    /// A YAML document does not follow the expected `name: {code, message}` shape.
    #[error("response file format invalid")]
    Format,

    /// A file name cannot be turned into a valid enum name.
    #[error("response file name invalid")]
    File,
}

/// Generate response enums from YAML files.
///
/// Walks `input` (relative to the crate root) and writes one formatted Rust enum per
/// YAML file into `OUT_DIR/output`. Each YAML entry is `name: {code, message}`; the
/// enum is named after the file stem (e.g. `general.yml` → `GeneralResponse`).
///
/// - `import_prefix`: prefix prepended to the `actix_cloud` import, e.g. `crate::` when
///   re-exported by the calling crate.
/// - `output`: generated file name inside `OUT_DIR`, included via `include!`.
///
/// This function should be used in `build.rs`.
/// ```ignore
/// [build-dependencies]
/// actix-cloud = { version = "xx", features = ["response-build"] }
/// ```
///
/// ```no_run
/// use actix_cloud::response_build::generate_response;
///
/// generate_response("", "response", "response.rs").unwrap();
/// ```
pub fn generate_response(import_prefix: &str, input: &str, output: &str) -> Result<()> {
    let outfile = Path::new(&env::var("OUT_DIR")?).join(output);
    let mut output = File::create(&outfile)?;
    writeln!(
        output,
        "use {}actix_cloud::response::ResponseCodeTrait;",
        import_prefix
    )?;
    for entry in WalkDir::new(input) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let file = read_to_string(entry.path())?;
            let yaml = YamlLoader::load_from_str(&file)?;
            let doc = &yaml[0];
            let mut name_vec = Vec::new();
            let mut code_vec = Vec::new();
            let mut message_vec = Vec::new();
            for (name, field) in doc.as_hash().ok_or(BuildError::Format)? {
                name_vec.push(format_ident!(
                    "{}",
                    name.as_str().ok_or(BuildError::Format)?
                ));
                code_vec.push(field["code"].as_i64().ok_or(BuildError::Format)?);
                message_vec.push(field["message"].as_str().ok_or(BuildError::Format)?);
            }

            let file_stem = entry.path().file_stem().ok_or(BuildError::File)?;
            let stem = file_stem.to_str().ok_or(BuildError::File)?;
            if stem.is_empty()
                || !stem.starts_with(|c: char| c.is_alphabetic() || c == '_')
                || !stem.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                return Err(BuildError::File.into());
            }
            let mut chars = stem.chars();
            let s = chars.next().unwrap().to_uppercase().collect::<String>() + chars.as_str();
            let enum_name = format_ident!("{}Response", s);
            let mut enum_code = Vec::new();
            for i in 0..code_vec.len() {
                let s = &name_vec[i];
                let c = code_vec[i];
                enum_code.push(quote! {#enum_name::#s => #c});
            }
            let mut enum_message = Vec::new();
            for i in 0..code_vec.len() {
                let s = &name_vec[i];
                let c = message_vec[i];
                enum_message.push(quote! {#enum_name::#s => #c});
            }
            let content = quote! {
                pub enum #enum_name {
                    #(#name_vec),*
                }

                impl ResponseCodeTrait for #enum_name {
                    fn code(&self) -> i64 {
                        match self {
                            #(#enum_code),*
                        }
                    }

                    fn message(&self) -> &'static str {
                        match self {
                            #(#enum_message),*
                        }
                    }
                }
            };

            write!(
                output,
                "{}",
                prettyplease::unparse(&syn::parse_file(&content.to_string())?)
            )?;
        }
    }
    Ok(())
}
