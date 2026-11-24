#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use hypolia_lobby::auth::{AuthManager, PlayerSession, RefreshToken, UserClaims};
use hypolia_lobby::config::{EnvConfig, LayoutStore, LobbyLayout, WaypointPose};
use hypolia_lobby::storage::{
    Backoff, LOBBY_REALM, LayoutBackend, LayoutRepository, SessionBackend, SessionRepository,
    WriteGate,
};
use uuid::Uuid;

pub fn temp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("hypolia_lobby_test_{}", Uuid::new_v4()))
}

pub fn layout_with_spawn(x: f64) -> LobbyLayout {
    let mut layout = LobbyLayout::default();
    let pose = WaypointPose::new(x, 70.0, -3.5, 90.0, 5.0);
    layout.spawn = pose;
    layout.pois.insert("spawn".to_owned(), pose);
    layout
}

pub fn session(player_uuid: Uuid, refresh: &str) -> PlayerSession {
    PlayerSession {
        player_uuid,
        claims: UserClaims {
            sub: "user-1".to_owned(),
            username: "ferris".to_owned(),
            display_name: Some("Ferris".to_owned()),
            email: None,
            roles: vec!["member".to_owned()],
            exp: Some(1_999_999_999),
        },
        access_token: "access-1".to_owned(),
        refresh_token: Some(RefreshToken::new(refresh)),
        expires_at: 1_999_999_999,
    }
}

pub fn same_session(left: &PlayerSession, right: &PlayerSession) -> bool {
    serde_json::to_value(left).expect("left") == serde_json::to_value(right).expect("right")
}

pub async fn layout_contract<R: LayoutRepository>(repo: &R) {
    assert!(repo.load().await.expect("load empty").is_none());

    let first = layout_with_spawn(1.5);
    repo.save(&first).await.expect("save first");
    assert_eq!(repo.load().await.expect("load first"), Some(first));

    let second = layout_with_spawn(2.5);
    repo.save(&second).await.expect("save second");
    assert_eq!(
        repo.load().await.expect("load second"),
        Some(second),
        "last write wins"
    );
}

pub async fn session_contract<R: SessionRepository>(repo: &R) {
    let player = Uuid::new_v4();
    assert!(repo.get(player, LOBBY_REALM).await.expect("get").is_none());

    let first = session(player, "rt-1");
    repo.put(LOBBY_REALM, &first).await.expect("put");
    let loaded = repo
        .get(player, LOBBY_REALM)
        .await
        .expect("get after put")
        .expect("present");
    assert!(same_session(&loaded, &first));

    let second = session(player, "rt-2");
    repo.put(LOBBY_REALM, &second).await.expect("overwrite");
    let loaded = repo
        .get(player, LOBBY_REALM)
        .await
        .expect("get after overwrite")
        .expect("present");
    assert!(same_session(&loaded, &second));

    let other = Uuid::new_v4();
    assert!(repo.get(other, LOBBY_REALM).await.expect("other").is_none());

    repo.delete(player, LOBBY_REALM).await.expect("delete");
    repo.delete(player, LOBBY_REALM)
        .await
        .expect("delete is idempotent");
    assert!(
        repo.get(player, LOBBY_REALM)
            .await
            .expect("get after delete")
            .is_none()
    );
}

pub async fn layout_survives_restart<F>(make: F)
where
    F: Fn() -> LayoutBackend,
{
    let seed = LobbyLayout::default();
    let store = LayoutStore::open(
        make(),
        &seed,
        Arc::new(WriteGate::default()),
        Backoff::startup(),
    )
    .await
    .expect("first start");
    assert_eq!(store.snapshot(), seed);

    let admin_pose = WaypointPose::new(42.5, 80.0, 7.5, 180.0, 0.0);
    store
        .update(|layout| {
            layout.spawn = admin_pose;
            layout.pois.insert("spawn".to_owned(), admin_pose);
            Ok(())
        })
        .await
        .expect("setspawn");
    drop(store);

    let restarted = LayoutStore::open(
        make(),
        &seed,
        Arc::new(WriteGate::default()),
        Backoff::startup(),
    )
    .await
    .expect("restart");
    assert_eq!(restarted.snapshot().spawn, admin_pose);
}

pub async fn session_survives_restart<F>(make: F)
where
    F: Fn() -> SessionBackend,
{
    let player = Uuid::new_v4();
    let stored = session(player, "rt-persisted");
    make()
        .put(LOBBY_REALM, &stored)
        .await
        .expect("put before restart");

    let manager = AuthManager::with_backends(
        EnvConfig::from_env(),
        make(),
        make(),
        Arc::new(WriteGate::default()),
    );
    assert!(!manager.is_authenticated(player));
    manager.hydrate(player).await;
    assert!(manager.is_authenticated(player));
    assert_eq!(
        manager.claims_of(player).map(|claims| claims.username),
        Some("ferris".to_owned())
    );
}
