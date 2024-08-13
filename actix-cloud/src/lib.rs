pub use actix_cloud_codegen::main;
pub use actix_web;
#[cfg(feature = "logger")]
pub use tracing;

pub mod error;
#[cfg(feature = "i18n")]
pub mod i18n;
pub mod memorydb;
pub mod router;
pub mod security;
#[cfg(feature = "session")]
pub mod session;
pub mod state;
pub mod utils;

#[cfg(feature = "logger")]
pub mod logger;

pub use error::Result;
pub use router::build_router;

/// Make map creation easier.
///
/// # Examples
///
/// ```
/// use actix_cloud::map;
/// let val = map!["key" => "value"];
/// ```
#[macro_export]
macro_rules! map {
    {$($key:expr => $value:expr),+} => {{
        let mut m = std::collections::HashMap::new();
        $(
            m.insert($key, $value);
        )+
        m
    }};
}
