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

#[derive(thiserror::Error, Debug)]
pub enum BuildError {
    #[error("response file format invalid")]
    Format,

    #[error("response file name invalid")]
    File,
}

/// Generate response file from yml.
///
/// This function should be used in `build.rs`.
/// ```ignore
/// [build-dependencies]
/// actix-cloud = { version = "xx", features = ["response-build"] }
/// ```
///
/// ```no_run
/// use actix_cloud::response::generate_response;
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
            let mut s = file_stem.to_str().ok_or(BuildError::File)?.to_owned();
            let s = s.remove(0).to_uppercase().to_string() + &s;
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
