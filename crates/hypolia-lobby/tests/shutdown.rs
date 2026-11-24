use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

mod common;

fn wait_for_exit(
    child: &mut std::process::Child,
    limit: Duration,
) -> Option<std::process::ExitStatus> {
    let started = Instant::now();
    while started.elapsed() < limit {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[test]
fn plc_8_binary_exits_zero_within_five_seconds_on_sigterm() {
    let dir = common::temp_dir();
    std::fs::create_dir_all(&dir).expect("dir");
    let mut child = Command::new(env!("CARGO_BIN_EXE_hypolia-lobby"))
        .env("LOBBY_LAYOUT_PATH", dir.join("lobby.json"))
        .env("LOBBY_SESSIONS_PATH", dir.join("sessions.json"))
        .env("CAPITAINERIE_SESSIONS_PATH", dir.join("cap.json"))
        .env("RSPIGOT_URL", "http://127.0.0.1:1")
        .env_remove("LOBBY_DATABASE_URL")
        .env_remove("POD_NAME")
        .env_remove("ORCHESTRATOR_URL")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn lobby");

    let layout_file = dir.join("lobby.json");
    let ready = Instant::now();
    while !layout_file.exists() && ready.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(layout_file.exists(), "le plugin n'a pas démarré");
    std::thread::sleep(Duration::from_millis(300));

    let pid = child.id().to_string();
    let signalled = Command::new("kill")
        .args(["-TERM", &pid])
        .status()
        .expect("kill");
    assert!(signalled.success());

    let sent = Instant::now();
    let status = wait_for_exit(&mut child, Duration::from_secs(5));
    let elapsed = sent.elapsed();
    if status.is_none() {
        let _ = child.kill();
    }
    let _ = std::fs::remove_dir_all(&dir);
    let status = status.expect("le plugin doit sortir en moins de 5 s");
    assert_eq!(status.code(), Some(0));
    assert!(elapsed < Duration::from_secs(5));
}
