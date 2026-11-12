//! The loft binary as a process: what it does with arguments it doesn't know.

#[path = "../src/testdir.rs"]
#[allow(dead_code)]
mod testdir;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn an_unknown_argument_exits_before_opening_the_db_or_a_port() {
    let root = testdir::TempDir::new("args");
    // A port nothing else uses, so a server that did start would be seen (and stopped) rather than collide.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("some-later-subcommand")
        .env("LOFT_ROOT", &*root)
        .env("LOFT_PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > end {
            child.kill().unwrap();
            panic!("loft kept running with an unknown argument");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!status.success());
    assert!(stderr.contains("unknown argument \"some-later-subcommand\""), "{stderr}");
    assert!(!root.join(".loft").exists(), "it opened the db");
    assert!(!root.join("agent-local-storage").exists());
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err(), "it bound the port");
}
