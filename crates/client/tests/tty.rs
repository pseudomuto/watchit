use std::process::{Command, Stdio};

#[test]
fn client_without_a_terminal_fails_with_a_clear_error() {
    let out = Command::new(env!("CARGO_BIN_EXE_client"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(stderr.contains("needs a terminal"), "unexpected stderr:\n{stderr}");
}
