use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};

fn vem() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vem"))
}

#[test]
fn serve_binds_loopback_and_answers_the_api() {
    let tmp = tempfile::tempdir().unwrap();
    let case = tmp.path().join("case");
    assert!(vem()
        .args(["case", "new", case.to_str().unwrap(), "--name", "served"])
        .status()
        .unwrap()
        .success());
    let mut child = vem()
        .args(["serve", case.to_str().unwrap(), "--port", "0", "--no-open"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let url = line
        .split_whitespace()
        .find(|w| w.starts_with("http://"))
        .unwrap_or_else(|| panic!("no URL in {line:?}"))
        .to_string();
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    let addr = url
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
    let mut s = TcpStream::connect(&addr).unwrap();
    write!(
        s,
        "GET /api/case HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut resp = String::new();
    s.read_to_string(&mut resp).unwrap();
    child.kill().unwrap();
    let _ = child.wait();
    assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
    assert!(resp.contains("\"served\""));
}

#[test]
fn serve_refuses_a_directory_that_is_not_a_case() {
    let tmp = tempfile::tempdir().unwrap();
    let out = vem()
        .args([
            "serve",
            tmp.path().to_str().unwrap(),
            "--port",
            "0",
            "--no-open",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a case directory"));
}
