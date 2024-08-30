use std::{
    collections::HashSet,
    env,
    hash::Hash,
    process::{Child, Command},
};

use rand::{
    distributions::{Alphanumeric, Uniform},
    thread_rng, Rng,
};

use crate::Result;

pub fn is_unique<T>(iter: T) -> bool
where
    T: IntoIterator,
    T::Item: Eq + Hash,
{
    let mut uniq = HashSet::new();
    iter.into_iter().all(move |x| uniq.insert(x))
}

pub fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    *t == Default::default()
}

/// Get `n` bytes random string.
/// `[a-zA-Z0-9]+`
pub fn rand_string(n: usize) -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(n)
        .map(char::from)
        .collect()
}

/// Get `n` bytes random hex string.
/// `[a-f0-9]+`
pub fn rand_string_hex(n: usize) -> String {
    let mut rng = thread_rng();
    let bytes: Vec<u8> = (0..n / 2).map(|_| rng.gen()).collect();
    hex::encode(bytes)
}

/// Get `n` bytes random string (all printable ascii).
pub fn rand_string_all(n: usize) -> String {
    thread_rng()
        .sample_iter(Uniform::new(char::from(33), char::from(126)))
        .take(n)
        .map(char::from)
        .collect()
}

/// Restart the program and keep the argument.
///
/// Inherit the environment/io/working directory of current process.
pub fn restart() -> Result<Child> {
    Command::new(env::current_exe().unwrap())
        .args(env::args().skip(1))
        .spawn()
        .map_err(Into::into)
}

#[cfg(feature = "rustls")]
pub fn load_rustls_config<P: AsRef<std::path::Path>>(
    cert: P,
    key: P,
) -> Result<rustls::ServerConfig> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .unwrap();
    let config = rustls::ServerConfig::builder().with_no_client_auth();
    let cert_chain =
        rustls_pemfile::certs(&mut std::io::BufReader::new(std::fs::File::open(cert)?))
            .collect::<Result<Vec<_>, _>>()?;
    let mut key_chain =
        rustls_pemfile::pkcs8_private_keys(&mut std::io::BufReader::new(std::fs::File::open(key)?))
            .map(|v| v.map(rustls::pki_types::PrivateKeyDer::Pkcs8))
            .collect::<Result<Vec<_>, _>>()?;

    let Some(private_key) = key_chain.pop() else {
        anyhow::bail!("Cannot find PKCS 8 private key");
    };

    config
        .with_single_cert(cert_chain, private_key)
        .map_err(Into::into)
}
