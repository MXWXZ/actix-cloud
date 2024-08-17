use std::io;

use actix_cloud::{
    i18n::{i18n, Locale},
    t,
};

#[actix_cloud::main]
async fn main() -> io::Result<()> {
    let locale = Locale::new(String::from("en-US")).add_locale(i18n!("locale"));

    println!("Default: {}", t!(locale, "hello.world"));
    println!("Translated: {}", t!(locale, "hello.world", "zh-CN"));
    println!("Param: {}", t!(locale, "hello.name", name = "MEME"));
    println!(
        "Param translated: {}",
        t!(locale, "hello.name", "zh-CN", name = "MEME")
    );
    println!("Fallback: {}", t!(locale, "english.only", "zh-CN"));
    println!("Not exist default: {}", t!(locale, "not.exist"));
    println!("Not exist translate: {}", t!(locale, "not.exist", "zh-CN"));
    Ok(())
}
