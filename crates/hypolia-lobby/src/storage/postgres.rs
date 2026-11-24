use std::time::Duration;

use sqlx::PgPool;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use super::{LayoutRepository, RepositoryError, SessionRepository};
use crate::auth::PlayerSession;
use crate::config::LobbyLayout;

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

const POOL_SIZE: u32 = 5;
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

fn map_sqlx(err: sqlx::Error) -> RepositoryError {
    match &err {
        sqlx::Error::Database(db) => match db.code().as_deref() {
            Some("23505" | "40001" | "40P01") => RepositoryError::Conflict(err.to_string()),
            _ => RepositoryError::Unavailable(err.to_string()),
        },
        sqlx::Error::ColumnDecode { .. } | sqlx::Error::Decode(_) => {
            RepositoryError::Corrupt("ligne illisible en base".to_owned())
        }
        _ => RepositoryError::Unavailable(err.to_string()),
    }
}

pub async fn connect_pool(url: &str) -> Result<PgPool, RepositoryError> {
    PgPoolOptions::new()
        .max_connections(POOL_SIZE)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect(url)
        .await
        .map_err(map_sqlx)
}

pub async fn migrate(pool: &PgPool) -> Result<(), RepositoryError> {
    MIGRATOR
        .run(pool)
        .await
        .map_err(|err| RepositoryError::Unavailable(format!("migration impossible : {err}")))
}

pub struct PgLayoutRepository {
    pool: PgPool,
}

impl PgLayoutRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl LayoutRepository for PgLayoutRepository {
    async fn load(&self) -> Result<Option<LobbyLayout>, RepositoryError> {
        let row: Option<(serde_json::Value,)> =
            sqlx::query_as("SELECT layout FROM lobby_layout WHERE id = 1")
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?;
        row.map(|(value,)| {
            serde_json::from_value::<LobbyLayout>(value)
                .map_err(|err| RepositoryError::Corrupt(format!("layout invalide en base : {err}")))
        })
        .transpose()
    }

    async fn save(&self, layout: &LobbyLayout) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(layout)
            .map_err(|err| RepositoryError::Corrupt(format!("layout non sérialisable : {err}")))?;
        sqlx::query(
            "INSERT INTO lobby_layout (id, layout, updated_at) VALUES (1, $1, now()) \
             ON CONFLICT (id) DO UPDATE SET layout = EXCLUDED.layout, updated_at = now()",
        )
        .bind(value)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }
}

pub struct PgSessionRepository {
    pool: PgPool,
}

impl PgSessionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl SessionRepository for PgSessionRepository {
    async fn get(
        &self,
        player_uuid: Uuid,
        realm: &str,
    ) -> Result<Option<PlayerSession>, RepositoryError> {
        let row: Option<(serde_json::Value,)> = sqlx::query_as(
            "SELECT session FROM player_sessions WHERE player_uuid = $1 AND realm = $2",
        )
        .bind(player_uuid)
        .bind(realm)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        row.map(|(value,)| {
            serde_json::from_value::<PlayerSession>(value).map_err(|_| {
                RepositoryError::Corrupt(format!("session illisible pour {player_uuid}"))
            })
        })
        .transpose()
    }

    async fn put(&self, realm: &str, session: &PlayerSession) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(session)
            .map_err(|_| RepositoryError::Corrupt("session non sérialisable".to_owned()))?;
        sqlx::query(
            "INSERT INTO player_sessions (player_uuid, realm, session, updated_at) \
             VALUES ($1, $2, $3, now()) \
             ON CONFLICT (player_uuid, realm) \
             DO UPDATE SET session = EXCLUDED.session, updated_at = now()",
        )
        .bind(session.player_uuid)
        .bind(realm)
        .bind(value)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn delete(&self, player_uuid: Uuid, realm: &str) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM player_sessions WHERE player_uuid = $1 AND realm = $2")
            .bind(player_uuid)
            .bind(realm)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }
}
