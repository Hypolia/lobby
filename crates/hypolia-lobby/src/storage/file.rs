use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use uuid::Uuid;

use super::{LayoutRepository, RepositoryError, SessionRepository};
use crate::auth::PlayerSession;
use crate::config::LobbyLayout;

pub struct FileLayoutRepository {
    path: PathBuf,
}

impl FileLayoutRepository {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn is_detached(&self) -> bool {
        self.path.as_os_str().is_empty()
    }
}

impl LayoutRepository for FileLayoutRepository {
    async fn load(&self) -> Result<Option<LobbyLayout>, RepositoryError> {
        if self.is_detached() || !self.path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&self.path).map_err(|err| {
            RepositoryError::Unavailable(format!(
                "lecture de {} impossible : {err}",
                self.path.display()
            ))
        })?;
        serde_json::from_str::<LobbyLayout>(&raw)
            .map(Some)
            .map_err(|err| {
                RepositoryError::Corrupt(format!(
                    "JSON de configuration invalide ({}) : {err}",
                    self.path.display()
                ))
            })
    }

    async fn save(&self, layout: &LobbyLayout) -> Result<(), RepositoryError> {
        if self.is_detached() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|err| {
                RepositoryError::Unavailable(format!(
                    "création de {} impossible : {err}",
                    parent.display()
                ))
            })?;
        }
        let pretty = serde_json::to_string_pretty(layout)
            .map_err(|err| RepositoryError::Corrupt(format!("layout non sérialisable : {err}")))?;
        std::fs::write(&self.path, format!("{pretty}\n")).map_err(|err| {
            RepositoryError::Unavailable(format!(
                "écriture de {} impossible : {err}",
                self.path.display()
            ))
        })
    }
}

pub struct FileSessionRepository {
    path: PathBuf,
    sessions: Mutex<HashMap<Uuid, PlayerSession>>,
}

impl FileSessionRepository {
    pub fn new(path: PathBuf) -> Self {
        let loaded = Self::load_sessions(&path);
        Self {
            path,
            sessions: Mutex::new(loaded),
        }
    }

    fn load_sessions(path: &Path) -> HashMap<Uuid, PlayerSession> {
        if path.as_os_str().is_empty() || !path.exists() {
            return HashMap::new();
        }
        let Ok(raw) = std::fs::read_to_string(path) else {
            tracing::warn!("Fichier de sessions illisible ({})", path.display());
            return HashMap::new();
        };
        match serde_json::from_str::<Vec<PlayerSession>>(&raw) {
            Ok(list) => list.into_iter().map(|s| (s.player_uuid, s)).collect(),
            Err(_) => {
                tracing::warn!("Fichier de sessions invalide ({})", path.display());
                HashMap::new()
            }
        }
    }

    fn persist(&self, sessions: &HashMap<Uuid, PlayerSession>) -> Result<(), RepositoryError> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        let list: Vec<&PlayerSession> = sessions.values().collect();
        let raw = serde_json::to_string_pretty(&list)
            .map_err(|_| RepositoryError::Corrupt("sessions non sérialisables".to_owned()))?;
        std::fs::write(&self.path, format!("{raw}\n")).map_err(|err| {
            RepositoryError::Unavailable(format!(
                "écriture de {} impossible : {err}",
                self.path.display()
            ))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

impl SessionRepository for FileSessionRepository {
    async fn get(
        &self,
        player_uuid: Uuid,
        _realm: &str,
    ) -> Result<Option<PlayerSession>, RepositoryError> {
        Ok(self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&player_uuid)
            .cloned())
    }

    async fn put(&self, _realm: &str, session: &PlayerSession) -> Result<(), RepositoryError> {
        let mut guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(session.player_uuid, session.clone());
        self.persist(&guard)
    }

    async fn delete(&self, player_uuid: Uuid, _realm: &str) -> Result<(), RepositoryError> {
        let mut guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if guard.remove(&player_uuid).is_some() {
            self.persist(&guard)?;
        }
        Ok(())
    }
}
