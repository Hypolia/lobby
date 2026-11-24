mod file;
mod gate;
mod postgres;
mod retry;

use std::future::Future;
use std::path::Path;

use thiserror::Error;
use uuid::Uuid;

use crate::auth::PlayerSession;
use crate::config::{EnvConfig, LobbyLayout};

pub use file::{FileLayoutRepository, FileSessionRepository};
pub use gate::WriteGate;
pub use postgres::{MIGRATOR, PgLayoutRepository, PgSessionRepository, connect_pool, migrate};
pub use retry::{Backoff, retry_unavailable};

pub const DATABASE_URL_ENV: &str = "LOBBY_DATABASE_URL";
pub const LOBBY_REALM: &str = "lobby";
pub const CAPITAINERIE_REALM: &str = "capitainerie";

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("stockage indisponible : {0}")]
    Unavailable(String),
    #[error("données de stockage corrompues : {0}")]
    Corrupt(String),
    #[error("conflit d'écriture : {0}")]
    Conflict(String),
}

pub trait LayoutRepository {
    fn load(&self) -> impl Future<Output = Result<Option<LobbyLayout>, RepositoryError>> + Send;

    fn save(
        &self,
        layout: &LobbyLayout,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
}

pub trait SessionRepository {
    fn get(
        &self,
        player_uuid: Uuid,
        realm: &str,
    ) -> impl Future<Output = Result<Option<PlayerSession>, RepositoryError>> + Send;

    fn put(
        &self,
        realm: &str,
        session: &PlayerSession,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    fn delete(
        &self,
        player_uuid: Uuid,
        realm: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
}

pub enum LayoutBackend {
    File(FileLayoutRepository),
    Postgres(PgLayoutRepository),
}

impl LayoutRepository for LayoutBackend {
    async fn load(&self) -> Result<Option<LobbyLayout>, RepositoryError> {
        match self {
            Self::File(repo) => repo.load().await,
            Self::Postgres(repo) => repo.load().await,
        }
    }

    async fn save(&self, layout: &LobbyLayout) -> Result<(), RepositoryError> {
        match self {
            Self::File(repo) => repo.save(layout).await,
            Self::Postgres(repo) => repo.save(layout).await,
        }
    }
}

pub enum SessionBackend {
    File(FileSessionRepository),
    Postgres(PgSessionRepository),
}

impl SessionRepository for SessionBackend {
    async fn get(
        &self,
        player_uuid: Uuid,
        realm: &str,
    ) -> Result<Option<PlayerSession>, RepositoryError> {
        match self {
            Self::File(repo) => repo.get(player_uuid, realm).await,
            Self::Postgres(repo) => repo.get(player_uuid, realm).await,
        }
    }

    async fn put(&self, realm: &str, session: &PlayerSession) -> Result<(), RepositoryError> {
        match self {
            Self::File(repo) => repo.put(realm, session).await,
            Self::Postgres(repo) => repo.put(realm, session).await,
        }
    }

    async fn delete(&self, player_uuid: Uuid, realm: &str) -> Result<(), RepositoryError> {
        match self {
            Self::File(repo) => repo.delete(player_uuid, realm).await,
            Self::Postgres(repo) => repo.delete(player_uuid, realm).await,
        }
    }
}

pub struct Backends {
    pub layout: LayoutBackend,
    pub lobby_sessions: SessionBackend,
    pub capitainerie_sessions: SessionBackend,
}

impl Backends {
    pub fn file(config: &EnvConfig) -> Self {
        let capitainerie_path = if config.has_dedicated_capitainerie_auth() {
            config.capitainerie_sessions_path.clone()
        } else {
            std::path::PathBuf::new()
        };
        Self {
            layout: LayoutBackend::File(FileLayoutRepository::new(config.layout_path.clone())),
            lobby_sessions: SessionBackend::File(FileSessionRepository::new(
                config.sessions_path.clone(),
            )),
            capitainerie_sessions: SessionBackend::File(FileSessionRepository::new(
                capitainerie_path,
            )),
        }
    }

    pub async fn postgres(url: &str, backoff: Backoff) -> Result<Self, RepositoryError> {
        let pool = retry_unavailable(backoff, "connexion Postgres", || connect_pool(url)).await?;
        Ok(Self {
            layout: LayoutBackend::Postgres(PgLayoutRepository::new(pool.clone())),
            lobby_sessions: SessionBackend::Postgres(PgSessionRepository::new(pool.clone())),
            capitainerie_sessions: SessionBackend::Postgres(PgSessionRepository::new(pool)),
        })
    }

    pub async fn open(config: &EnvConfig, backoff: Backoff) -> Result<Self, RepositoryError> {
        match database_url() {
            Some(url) => Self::postgres(&url, backoff).await,
            None => Ok(Self::file(config)),
        }
    }
}

pub fn database_url() -> Option<String> {
    std::env::var(DATABASE_URL_ENV)
        .ok()
        .map(|raw| raw.trim().to_owned())
        .filter(|val| !val.is_empty())
}

pub fn bundled_layout(path: &Path) -> Result<LobbyLayout, RepositoryError> {
    if !path.exists() {
        return Ok(LobbyLayout::default());
    }
    let raw = std::fs::read_to_string(path).map_err(|err| {
        RepositoryError::Unavailable(format!("lecture de {} impossible : {err}", path.display()))
    })?;
    serde_json::from_str::<LobbyLayout>(&raw).map_err(|_| {
        RepositoryError::Corrupt(format!("{} n'est pas un layout valide", path.display()))
    })
}

pub async fn load_or_seed<R: LayoutRepository>(
    repo: &R,
    seed: &LobbyLayout,
    backoff: Backoff,
) -> Result<LobbyLayout, RepositoryError> {
    let stored = retry_unavailable(backoff, "chargement du layout", || repo.load()).await?;
    match stored {
        Some(mut layout) => {
            if add_missing_towerwars_pedestal(&mut layout, seed) {
                retry_unavailable(backoff, "migration du layout", || repo.save(&layout)).await?;
                tracing::info!("Piédestal TowerWars ajouté au layout stocké");
            }
            Ok(layout)
        }
        None => {
            retry_unavailable(backoff, "initialisation du layout", || repo.save(seed)).await?;
            tracing::info!("Layout initialisé depuis la configuration embarquée");
            Ok(seed.clone())
        }
    }
}

fn add_missing_towerwars_pedestal(layout: &mut LobbyLayout, seed: &LobbyLayout) -> bool {
    const ID: &str = "towerwars";
    if layout.pedestals.iter().any(|p| p.id == ID) {
        return false;
    }
    let Some(wanted) = seed.pedestals.iter().find(|p| p.id == ID) else {
        return false;
    };
    if let Some(legacy) = layout
        .pedestals
        .iter_mut()
        .find(|p| p.id == "uhc" && !p.active)
    {
        let mut upgraded = wanted.clone();
        upgraded.index = legacy.index;
        upgraded.pose = legacy.pose;
        *legacy = upgraded;
        return true;
    }
    if layout.pedestals.iter().any(|p| p.index == wanted.index) {
        return false;
    }
    layout.pedestals.push(wanted.clone());
    true
}
