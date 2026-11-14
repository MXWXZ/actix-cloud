use std::{
    collections::HashSet,
    env,
    hash::Hash,
    io,
    process::{Child, Command},
};

#[cfg(feature = "rustls")]
use std::{fs, path};

use rand::{
    distr::{Alphanumeric, Uniform},
    rng, RngExt as _,
};

/// Check whether iterator `iter` contains only unique values.
pub fn is_unique<T>(iter: T) -> bool
where
    T: IntoIterator,
    T::Item: Eq + Hash,
{
    let mut uniq = HashSet::new();
    iter.into_iter().all(move |x| uniq.insert(x))
}

/// Check whether `t` equals the default value of `T`.
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

/// Restart the current program with the same command line arguments.
///
/// Inherits the environment, stdio and working directory of the current process.
pub fn restart() -> io::Result<Child> {
    Command::new(env::current_exe()?)
        .args(env::args().skip(1))
        .spawn()
}

/// Errors returned by [`load_rustls_config`].
#[cfg(feature = "rustls")]
#[derive(Debug, thiserror::Error)]
pub enum RustlsConfigError {
    /// The private key file contains no PKCS 8 private key.
    #[error("cannot find PKCS 8 private key")]
    MissingPrivateKey,
    /// Reading the certificate or private key file failed.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// rustls rejected the certificate and private key.
    #[error(transparent)]
    Rustls(#[from] rustls::Error),
}

#[cfg(feature = "rustls")]
/// Load SSL certificate and key.
///
/// Only PKCS 8 private keys are supported (`BEGIN PRIVATE KEY`). The aws-lc-rs
/// provider is installed as the default rustls crypto provider.
pub fn load_rustls_config<P: AsRef<path::Path>>(
    cert: P,
    key: P,
) -> Result<rustls::ServerConfig, RustlsConfigError> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let config = rustls::ServerConfig::builder().with_no_client_auth();
    let cert_chain = rustls_pemfile::certs(&mut io::BufReader::new(fs::File::open(cert)?))
        .collect::<Result<Vec<_>, _>>()?;
    let mut key_chain =
        rustls_pemfile::pkcs8_private_keys(&mut io::BufReader::new(fs::File::open(key)?))
            .map(|v| v.map(rustls::pki_types::PrivateKeyDer::Pkcs8))
            .collect::<Result<Vec<_>, _>>()?;

    let Some(private_key) = key_chain.pop() else {
        return Err(RustlsConfigError::MissingPrivateKey);
    };

    Ok(config.with_single_cert(cert_chain, private_key)?)
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
        for n in [0, 1, 2, 10, 63, 64, 4000] {
            let s = rand_string_all(n);
            assert_eq!(s.len(), n);
            assert!(s.chars().all(|c| ('!'..='~').contains(&c)));
        }
    }

    #[test]
    fn random_string_helpers_do_not_repeat() {
        use std::collections::HashSet;

        // Basic entropy sanity check for helpers used to generate tokens/secrets.
        let alnum: HashSet<String> = (0..256).map(|_| rand_string(32)).collect();
        let hex: HashSet<String> = (0..256).map(|_| rand_string_hex(32)).collect();
        let all: HashSet<String> = (0..256).map(|_| rand_string_all(32)).collect();

        assert_eq!(alnum.len(), 256);
        assert_eq!(hex.len(), 256);
        assert_eq!(all.len(), 256);
    }

    #[cfg(feature = "rustls")]
    #[test]
    fn load_rustls_config_reports_io_and_missing_key_errors() {
        let dir = std::env::temp_dir().join(format!("actix-cloud-rustls-{}", rand_string(12)));
        std::fs::create_dir_all(&dir).unwrap();

        let missing = dir.join("missing.pem");
        assert!(matches!(
            load_rustls_config(&missing, &missing),
            Err(RustlsConfigError::Io(_))
        ));

        let cert = dir.join("cert.pem");
        let key = dir.join("key.pem");
        std::fs::write(&cert, "").unwrap();
        std::fs::write(&key, "").unwrap();
        assert!(matches!(
            load_rustls_config(&cert, &key),
            Err(RustlsConfigError::MissingPrivateKey)
        ));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
