use std::{
    collections::HashSet,
    env,
    hash::Hash,
    process::{Child, Command},
};

use rand::{
    distr::{Alphanumeric, Uniform},
    rng, RngExt as _,
};

use crate::Result;

/// Check whether iterator `iter` contains only unique values.
pub fn is_unique<T>(iter: T) -> bool
where
    T: IntoIterator,
    T::Item: Eq + Hash,
{
    let mut uniq = HashSet::new();
    iter.into_iter().all(move |x| uniq.insert(x))
}

/// Check whether `t` type `T` is equal to default.
pub fn is_default<T: Default + PartialEq>(t: &T) -> bool {
    *t == Default::default()
}

/// Get `n` bytes random string.
/// `[a-zA-Z0-9]+`
pub fn rand_string(n: usize) -> String {
    rng()
        .sample_iter(&Alphanumeric)
        .take(n)
        .map(char::from)
        .collect()
}

/// Get `n` bytes random hex string.
/// `[a-f0-9]+`
pub fn rand_string_hex(n: usize) -> String {
    let mut rng = rng();
    let bytes: Vec<u8> = (0..n.div_ceil(2)).map(|_| rng.random()).collect();
    let mut s = hex::encode(bytes);
    s.truncate(n);
    s
}

/// Get `n` bytes random string (all printable ascii).
pub fn rand_string_all(n: usize) -> String {
    rng()
        .sample_iter(Uniform::new(char::from(33), char::from(127)).unwrap())
        .take(n)
        .collect()
}

/// Restart the program and keep the argument.
///
/// Inherit the environment/io/working directory of current process.
pub fn restart() -> Result<Child> {
    Command::new(env::current_exe()?)
        .args(env::args().skip(1))
        .spawn()
        .map_err(Into::into)
}

#[cfg(feature = "rustls")]
/// Load SSL certificate and key.
pub fn load_rustls_config<P: AsRef<std::path::Path>>(
    cert: P,
    key: P,
) -> Result<rustls::ServerConfig> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_unique() {
        assert!(is_unique(vec![1, 2, 3]));
        assert!(is_unique([1, 2, 3]));
        assert!(is_unique(vec![1]));
        assert!(is_unique(Vec::<i32>::new()));
        assert!(!is_unique(vec![1, 2, 2]));
        assert!(!is_unique([1, 1]));
        assert!(is_unique("abc".chars()));
        assert!(!is_unique("aba".chars()));
        assert!(is_unique("".chars()));
    }

    #[test]
    fn test_is_default() {
        assert!(is_default(&0_u8));
        assert!(is_default(&0_i64));
        assert!(!is_default(&1_i64));
        assert!(is_default(&String::new()));
        assert!(!is_default(&"a".to_string()));
        assert!(is_default(&Vec::<i32>::new()));
        assert!(!is_default(&vec![1]));
        assert!(is_default(&Option::<i32>::None));
        assert!(!is_default(&Some(1)));
    }

    #[test]
    fn test_rand_string() {
        for n in [0, 1, 2, 3, 4, 5, 10, 63, 64] {
            let s = rand_string(n);
            assert_eq!(s.len(), n);
            assert!(s.chars().all(|c| c.is_ascii_alphanumeric()));
        }
    }

    #[test]
    fn test_rand_string_hex() {
        for n in [0, 1, 2, 3, 4, 5, 10, 63, 64] {
            let s = rand_string_hex(n);
            assert_eq!(s.len(), n);
            assert!(s.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn test_rand_string_all() {
        let s = rand_string_all(4000);
        assert!(s.chars().all(|c| ('!'..='~').contains(&c)),);
    }
}
