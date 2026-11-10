use std::{fs, io, os::unix::fs::PermissionsExt, sync::Arc};

use rustls::{
    RootCertStore,
    client::{WebPkiServerVerifier, danger::ServerCertVerifier},
    crypto::ring,
    pki_types::{CertificateDer, ServerName, UnixTime, pem::PemObject},
    server::WebPkiClientVerifier,
};
use shared::certs::*;

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
