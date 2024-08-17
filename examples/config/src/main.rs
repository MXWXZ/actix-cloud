use actix_cloud::config;
use actix_cloud::config::Config;
use actix_cloud::Result;
use serde::Deserialize;
use serde_inline_default::serde_inline_default;
use validator::Validate;

#[serde_inline_default]
#[derive(Deserialize, Debug, Validate)]
struct Priority {
    #[serde_inline_default(42)]
    #[validate(range(min = 0, max = 64))]
    priority: u32,
    #[validate(length(min = 1, max = 16))]
    key: String,
}

#[derive(Deserialize, Debug, Validate)]
struct Setting {
    #[allow(dead_code)]
    debug: Option<bool>,
    #[validate(nested)]
    sub: Priority,
}

fn main() {
    let cfg = Config::builder()
        .add_source(config::File::with_name("setting.yml"))
        .build()
        .unwrap();
    let setting: Setting = cfg.try_deserialize().unwrap();
    println!("Normal: {:?}", setting);

    let cfg = Config::builder()
        .add_source(config::File::with_name("setting_optional.yml"))
        .build()
        .unwrap();
    let setting: Setting = cfg.try_deserialize().unwrap();
    println!("Optional: {:?}", setting);

    let cfg = Config::builder()
        .add_source(config::File::with_name("setting_invalid.yml"))
        .build()
        .unwrap();
    let setting: Result<Setting> = cfg.try_deserialize().map_err(Into::into);
    println!("Invalid: {:?}", setting);

    let cfg = Config::builder()
        .add_source(config::File::with_name("setting_validate.yml"))
        .build()
        .unwrap();
    let setting: Setting = cfg.try_deserialize().unwrap();
    println!("Validate: {:?}", setting.validate());
}
