use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

use rcgen::{
    BasicConstraints,
    CertificateParams,
    DistinguishedName,
    DnType,
    ExtendedKeyUsagePurpose,
    IsCa,
    Issuer,
    KeyPair,
    KeyUsagePurpose,
};
use time::{Duration, OffsetDateTime};

const CA_VALIDITY: Duration = Duration::days(3650);
const LEAF_VALIDITY: Duration = Duration::days(365);

// Certs are generated on one machine and verified on another, so backdate
// `not_before` a little to tolerate clock skew.
const CLOCK_SKEW: Duration = Duration::hours(1);

/// A PEM-encoded certificate and its private key.
#[derive(Debug, Clone)]
pub struct Bundle {
    pub cert_pem: String,
    pub key_pem: String,
}

impl Bundle {
    /// Writes `<stem>.pem` and `<stem>.key` to `dir`. The key is only readable
    /// by its owner. Existing files are never overwritten.
    pub fn save(&self, dir: &Path, stem: &str) -> io::Result<()> {
        let cert = dir.join(format!("{stem}.pem"));
        let key = dir.join(format!("{stem}.key"));

        // Check both up front so we never leave a half-written pair behind.
        for path in [&cert, &key] {
            if path.try_exists()? {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} already exists", path.display()),
                ));
            }
        }

        write(&cert, &self.cert_pem, 0o644)?;
        write(&key, &self.key_pem, 0o600)
    }

    /// Reads `<stem>.pem` and `<stem>.key` from `dir`.
    pub fn load(dir: &Path, stem: &str) -> io::Result<Self> {
        Ok(Self {
            cert_pem: read(&dir.join(format!("{stem}.pem")))?,
            key_pem: read(&dir.join(format!("{stem}.key")))?,
        })
    }
}

/// Reads a PEM file, naming the path in any error.
pub fn read(path: &Path) -> io::Result<String> {
    fs::read_to_string(path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))
}

fn write(path: &Path, contents: &str, mode: u32) -> io::Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)?
        .write_all(contents.as_bytes())
}

/// Generates a new self-signed CA that can sign leaf certificates only.
pub fn generate_ca() -> Result<Bundle, rcgen::Error> {
    let mut params = params("watchit CA", &[], CA_VALIDITY)?;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];

    let key = KeyPair::generate()?;
    let cert = params.self_signed(&key)?;

    Ok(Bundle {
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
    })
}

/// Issues a server certificate valid for the given DNS names and/or IPs.
pub fn issue_server(ca: &Bundle, sans: &[String]) -> Result<Bundle, rcgen::Error> {
    let mut params = params("watchit server", sans, LEAF_VALIDITY)?;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];

    sign(&params, ca)
}

/// Issues a client certificate with the given common name.
pub fn issue_client(ca: &Bundle, name: &str) -> Result<Bundle, rcgen::Error> {
    let mut params = params(name, &[], LEAF_VALIDITY)?;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];

    sign(&params, ca)
}

fn params(common_name: &str, sans: &[String], validity: Duration) -> Result<CertificateParams, rcgen::Error> {
    let now = OffsetDateTime::now_utc();

    let mut params = CertificateParams::new(sans)?;
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, common_name);
    params.not_before = now - CLOCK_SKEW;
    params.not_after = now + validity;
    Ok(params)
}

fn sign(params: &CertificateParams, ca: &Bundle) -> Result<Bundle, rcgen::Error> {
    let issuer = Issuer::from_ca_cert_pem(&ca.cert_pem, KeyPair::from_pem(&ca.key_pem)?)?;
    let key = KeyPair::generate()?;
    let cert = params.signed_by(&key, &issuer)?;

    Ok(Bundle {
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
    })
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, sync::Arc};

    use rustls::{
        RootCertStore,
        client::{WebPkiServerVerifier, danger::ServerCertVerifier},
        crypto::ring,
        pki_types::{CertificateDer, ServerName, UnixTime, pem::PemObject},
        server::WebPkiClientVerifier,
    };

    use super::*;

    fn der(bundle: &Bundle) -> CertificateDer<'static> {
        CertificateDer::from_pem_slice(bundle.cert_pem.as_bytes()).unwrap()
    }

    fn roots(ca: &Bundle) -> Arc<RootCertStore> {
        let mut store = RootCertStore::empty();
        store.add(der(ca)).unwrap();
        Arc::new(store)
    }

    #[test]
    fn client_cert_is_accepted_by_client_verifier() {
        let ca = generate_ca().unwrap();
        let client = issue_client(&ca, "laptop").unwrap();

        let verifier = WebPkiClientVerifier::builder_with_provider(roots(&ca), Arc::new(ring::default_provider()))
            .build()
            .unwrap();

        verifier
            .verify_client_cert(&der(&client), &[], UnixTime::now())
            .unwrap();
    }

    #[test]
    fn server_cert_is_accepted_for_each_san() {
        let ca = generate_ca().unwrap();
        let server = issue_server(&ca, &["myhost.example".into(), "10.0.0.5".into()]).unwrap();

        let verifier = WebPkiServerVerifier::builder_with_provider(roots(&ca), Arc::new(ring::default_provider()))
            .build()
            .unwrap();

        for name in ["myhost.example", "10.0.0.5"] {
            let name = ServerName::try_from(name).unwrap();
            verifier
                .verify_server_cert(&der(&server), &[], &name, &[], UnixTime::now())
                .unwrap();
        }
    }

    #[test]
    fn server_cert_is_rejected_as_client_cert() {
        let ca = generate_ca().unwrap();
        let server = issue_server(&ca, &["myhost.example".into()]).unwrap();

        let verifier = WebPkiClientVerifier::builder_with_provider(roots(&ca), Arc::new(ring::default_provider()))
            .build()
            .unwrap();

        let err = verifier
            .verify_client_cert(&der(&server), &[], UnixTime::now())
            .unwrap_err();
        assert!(matches!(err, rustls::Error::InvalidCertificate(_)), "{err:?}");
    }

    #[test]
    fn save_writes_key_readable_by_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        generate_ca().unwrap().save(dir.path(), "ca").unwrap();

        let mode = fs::metadata(dir.path().join("ca.key")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn save_refuses_to_overwrite_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let original = generate_ca().unwrap();
        original.save(dir.path(), "ca").unwrap();

        let err = generate_ca().unwrap().save(dir.path(), "ca").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(dir.path().join("ca.key")).unwrap(), original.key_pem);
    }

    #[test]
    fn load_reads_back_a_saved_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let ca = generate_ca().unwrap();
        ca.save(dir.path(), "ca").unwrap();

        let loaded = Bundle::load(dir.path(), "ca").unwrap();
        assert_eq!(loaded.cert_pem, ca.cert_pem);
        assert_eq!(loaded.key_pem, ca.key_pem);
    }
}
