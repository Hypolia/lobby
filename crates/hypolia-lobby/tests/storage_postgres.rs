#![cfg(feature = "postgres-tests")]

mod common;

use hypolia_lobby::config::LobbyLayout;
use hypolia_lobby::storage::{
    Backoff, CAPITAINERIE_REALM, LOBBY_REALM, LayoutBackend, LayoutRepository, PgLayoutRepository,
    PgSessionRepository, RepositoryError, SessionBackend, SessionRepository, connect_pool,
    load_or_seed, migrate,
};
use sqlx::PgPool;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::ContainerAsync;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use uuid::Uuid;

async fn start() -> (ContainerAsync<Postgres>, PgPool) {
    let container = Postgres::default().start().await.expect("container");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pool = connect_pool(&url).await.expect("pool");
    migrate(&pool).await.expect("migrate");
    (container, pool)
}

#[tokio::test]
async fn postgres_layout_passes_the_contract() {
    let (_container, pool) = start().await;
    common::layout_contract(&PgLayoutRepository::new(pool)).await;
}

#[tokio::test]
async fn postgres_sessions_pass_the_contract() {
    let (_container, pool) = start().await;
    common::session_contract(&PgSessionRepository::new(pool)).await;
}

#[tokio::test]
async fn plc_1_postgres_layout_survives_a_new_repository_instance() {
    let (_container, pool) = start().await;
    common::layout_survives_restart(|| {
        LayoutBackend::Postgres(PgLayoutRepository::new(pool.clone()))
    })
    .await;
}

#[tokio::test]
async fn plc_1_postgres_layout_survives_a_new_pool() {
    let container = Postgres::default().start().await.expect("container");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    migrate(&connect_pool(&url).await.expect("pool"))
        .await
        .expect("migrate");
    common::layout_survives_restart(|| {
        let lazy = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect_lazy(&url)
            .expect("lazy pool");
        LayoutBackend::Postgres(PgLayoutRepository::new(lazy))
    })
    .await;
}

#[tokio::test]
async fn plc_2_postgres_session_survives_a_new_repository_instance() {
    let (_container, pool) = start().await;
    common::session_survives_restart(|| {
        SessionBackend::Postgres(PgSessionRepository::new(pool.clone()))
    })
    .await;
}

#[tokio::test]
async fn sessions_are_isolated_per_realm() {
    let (_container, pool) = start().await;
    let repo = PgSessionRepository::new(pool);
    let player = Uuid::new_v4();
    repo.put(LOBBY_REALM, &common::session(player, "rt-lobby"))
        .await
        .expect("put lobby");
    assert!(
        repo.get(player, CAPITAINERIE_REALM)
            .await
            .expect("get cap")
            .is_none()
    );
    repo.put(CAPITAINERIE_REALM, &common::session(player, "rt-cap"))
        .await
        .expect("put cap");
    repo.delete(player, LOBBY_REALM).await.expect("delete");
    assert!(
        repo.get(player, CAPITAINERIE_REALM)
            .await
            .expect("get")
            .is_some()
    );
}

#[tokio::test]
async fn empty_postgres_layout_is_seeded_from_the_bundled_one() {
    let (_container, pool) = start().await;
    let repo = PgLayoutRepository::new(pool);
    assert!(repo.load().await.expect("load").is_none());
    let seed = LobbyLayout::default();
    let loaded = load_or_seed(&repo, &seed, Backoff::startup())
        .await
        .expect("seed");
    assert_eq!(loaded, seed);
    assert_eq!(repo.load().await.expect("reload"), Some(seed));
}

#[tokio::test]
async fn corrupt_session_row_is_corrupt_and_does_not_leak_the_token() {
    let (_container, pool) = start().await;
    let player = Uuid::new_v4();
    sqlx::query("INSERT INTO player_sessions (player_uuid, realm, session) VALUES ($1, $2, $3)")
        .bind(player)
        .bind(LOBBY_REALM)
        .bind(serde_json::json!({"player_uuid": player, "refresh_token": 12345, "access_token": "rt-LEAK"}))
        .execute(&pool)
        .await
        .expect("insert");
    let err = PgSessionRepository::new(pool)
        .get(player, LOBBY_REALM)
        .await
        .expect_err("corrupt");
    assert!(matches!(err, RepositoryError::Corrupt(_)));
    assert!(!err.to_string().contains("rt-LEAK"));
}

#[tokio::test]
async fn unreachable_postgres_is_unavailable() {
    let err = connect_pool("postgres://postgres:postgres@127.0.0.1:1/postgres")
        .await
        .expect_err("unreachable");
    assert!(matches!(err, RepositoryError::Unavailable(_)));
}

const LEGACY_LAYOUT_LOAD: &str = "SELECT layout FROM lobby_layout WHERE id = 1";
const LEGACY_LAYOUT_SAVE: &str = "INSERT INTO lobby_layout (id, layout, updated_at) VALUES (1, $1, now()) ON CONFLICT (id) DO UPDATE SET layout = EXCLUDED.layout, updated_at = now()";
const LEGACY_SESSION_GET: &str =
    "SELECT session FROM player_sessions WHERE player_uuid = $1 AND realm = $2";
const LEGACY_SESSION_PUT: &str = "INSERT INTO player_sessions (player_uuid, realm, session, updated_at) VALUES ($1, $2, $3, now()) ON CONFLICT (player_uuid, realm) DO UPDATE SET session = EXCLUDED.session, updated_at = now()";
const LEGACY_SESSION_DELETE: &str =
    "DELETE FROM player_sessions WHERE player_uuid = $1 AND realm = $2";

#[tokio::test]
async fn plc_7_previous_plugin_queries_run_on_the_migrated_schema() {
    let (_container, pool) = start().await;
    let layout = serde_json::to_value(LobbyLayout::default()).expect("layout");
    sqlx::query(LEGACY_LAYOUT_SAVE)
        .bind(&layout)
        .execute(&pool)
        .await
        .expect("legacy layout save");
    let row: (serde_json::Value,) = sqlx::query_as(LEGACY_LAYOUT_LOAD)
        .fetch_one(&pool)
        .await
        .expect("legacy layout load");
    assert_eq!(row.0, layout);

    let player = Uuid::new_v4();
    let session = serde_json::to_value(common::session(player, "rt-n1")).expect("session");
    sqlx::query(LEGACY_SESSION_PUT)
        .bind(player)
        .bind(LOBBY_REALM)
        .bind(&session)
        .execute(&pool)
        .await
        .expect("legacy session put");
    let row: (serde_json::Value,) = sqlx::query_as(LEGACY_SESSION_GET)
        .bind(player)
        .bind(LOBBY_REALM)
        .fetch_one(&pool)
        .await
        .expect("legacy session get");
    assert_eq!(row.0, session);
    sqlx::query(LEGACY_SESSION_DELETE)
        .bind(player)
        .bind(LOBBY_REALM)
        .execute(&pool)
        .await
        .expect("legacy session delete");

    migrate(&pool).await.expect("migrations are idempotent");
}
