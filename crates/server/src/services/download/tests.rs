use tonic::Code;

use super::*;

#[test]
fn to_status_maps_deluge_errors() {
    let cases = [
        (
            deluge::Error::Status(reqwest::StatusCode::BAD_GATEWAY),
            Code::Unavailable,
        ),
        (deluge::Error::Auth, Code::FailedPrecondition),
        (deluge::Error::NoHosts, Code::FailedPrecondition),
        (
            deluge::Error::Rpc {
                code: 2,
                message: "Torrent already in session".into(),
            },
            Code::FailedPrecondition,
        ),
        (deluge::Error::Parse("bad".into()), Code::Internal),
        (deluge::Error::Uri, Code::InvalidArgument),
    ];

    for (err, code) in cases {
        assert_eq!(to_status("add", err).code(), code);
    }
}
