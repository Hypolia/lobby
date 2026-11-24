mod common;

use hypolia_lobby::config::LobbyLayout;
use hypolia_lobby::storage::{
    Backoff, FileLayoutRepository, FileSessionRepository, LOBBY_REALM, LayoutBackend,
    LayoutRepository, RepositoryError, SessionBackend, SessionRepository, load_or_seed,
};
use uuid::Uuid;

#[tokio::test]
async fn file_layout_passes_the_contract() {
    let dir = common::temp_dir();
    let repo = FileLayoutRepository::new(dir.join("lobby.json"));
    common::layout_contract(&repo).await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn file_sessions_pass_the_contract() {
    let dir = common::temp_dir();
    std::fs::create_dir_all(&dir).expect("dir");
    let repo = FileSessionRepository::new(dir.join("sessions.json"));
    common::session_contract(&repo).await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn plc_1_file_layout_survives_a_new_repository_instance() {
    let dir = common::temp_dir();
    let path = dir.join("lobby.json");
    common::layout_survives_restart(|| {
        LayoutBackend::File(FileLayoutRepository::new(path.clone()))
    })
    .await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn plc_2_file_session_survives_a_new_repository_instance() {
    let dir = common::temp_dir();
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("sessions.json");
    common::session_survives_restart(|| {
        SessionBackend::File(FileSessionRepository::new(path.clone()))
    })
    .await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn corrupt_layout_file_is_reported_as_corrupt() {
    let dir = common::temp_dir();
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("lobby.json");
    std::fs::write(&path, "{ not json").expect("write");
    let err = FileLayoutRepository::new(path)
        .load()
        .await
        .expect_err("corrupt");
    assert!(matches!(err, RepositoryError::Corrupt(_)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn corrupt_layout_is_never_replaced_by_the_seed() {
    let dir = common::temp_dir();
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("lobby.json");
    std::fs::write(&path, "{ not json").expect("write");
    let repo = FileLayoutRepository::new(path.clone());
    let err = load_or_seed(&repo, &LobbyLayout::default(), Backoff::startup())
        .await
        .expect_err("corrupt");
    assert!(matches!(err, RepositoryError::Corrupt(_)));
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "{ not json");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn session_file_is_private() {
    let dir = common::temp_dir();
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("sessions.json");
    let repo = FileSessionRepository::new(path.clone());
    let player = Uuid::new_v4();
    repo.put(LOBBY_REALM, &common::session(player, "rt-file"))
        .await
        .expect("put");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
