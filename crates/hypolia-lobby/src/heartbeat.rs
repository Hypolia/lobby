use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hypolia_protocol::{HeartbeatRequest, ServerKind, ServerPhase};
use rspigot_plugin::{Server, Uuid};
use tokio::time::MissedTickBehavior;

use crate::config::EnvConfig;
use crate::orchestrator::{OrchestratorClient, OrchestratorClientError};

pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
pub const FAILURE_LOG_EVERY: Duration = Duration::from_secs(30);
pub const FINAL_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(3);
const DEFAULT_MAX_PLAYERS: u32 = 100;
const DEFAULT_SERVER_PORT: u16 = 25565;

#[derive(Debug, Default)]
pub struct FailureLog {
    last_logged: Option<Instant>,
}

impl FailureLog {
    pub fn on_failure(&mut self, now: Instant) -> bool {
        let due = self
            .last_logged
            .is_none_or(|at| now.duration_since(at) >= FAILURE_LOG_EVERY);
        if due {
            self.last_logged = Some(now);
        }
        due
    }

    pub fn on_success(&mut self) -> bool {
        self.last_logged.take().is_some()
    }
}

#[derive(Debug, Clone)]
struct Observation {
    max_players: u32,
    player_uuids: Vec<Uuid>,
}

pub struct HeartbeatReporter {
    client: OrchestratorClient,
    server_id: String,
    address: Option<String>,
    version: Option<String>,
    observed: Mutex<Observation>,
    ending: AtomicBool,
}

impl HeartbeatReporter {
    pub fn new(
        client: OrchestratorClient,
        server_id: String,
        address: Option<String>,
        version: Option<String>,
    ) -> Self {
        Self {
            client,
            server_id,
            address,
            version,
            observed: Mutex::new(Observation {
                max_players: DEFAULT_MAX_PLAYERS,
                player_uuids: Vec::new(),
            }),
            ending: AtomicBool::new(false),
        }
    }

    pub fn from_env(config: &EnvConfig) -> Option<Self> {
        let url = config.orchestrator_url.clone()?;
        let server_id = env_non_empty("POD_NAME")?;
        let port = env_non_empty("SERVER_PORT")
            .and_then(|raw| raw.parse::<u16>().ok())
            .unwrap_or(DEFAULT_SERVER_PORT);
        let address = env_non_empty("POD_IP").map(|ip| format!("{ip}:{port}"));
        Some(Self::new(
            OrchestratorClient::new(url, config.orchestrator_token.clone()),
            server_id,
            address,
            crate::config::lobby_version(),
        ))
    }

    pub fn observe(&self, max_players: u32, player_uuids: Vec<Uuid>) {
        *self.observed.lock().unwrap_or_else(|e| e.into_inner()) = Observation {
            max_players,
            player_uuids,
        };
    }

    pub fn payload(&self, phase: ServerPhase) -> HeartbeatRequest {
        let observed = self
            .observed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        HeartbeatRequest {
            server_id: self.server_id.clone(),
            address: self.address.clone(),
            kind: ServerKind::Lobby,
            phase,
            player_count: u32::try_from(observed.player_uuids.len()).unwrap_or(u32::MAX),
            max_players: observed.max_players,
            player_uuids: observed.player_uuids,
            version: self.version.clone(),
        }
    }

    pub async fn beat(&self, phase: ServerPhase) -> Result<(), OrchestratorClientError> {
        self.client.send_heartbeat(&self.payload(phase)).await
    }

    pub async fn send_final(&self) {
        self.ending.store(true, Ordering::SeqCst);
        match tokio::time::timeout(FINAL_HEARTBEAT_TIMEOUT, self.beat(ServerPhase::Ending)).await {
            Ok(Ok(())) => {
                tracing::info!(server_id = %self.server_id, "Dernier heartbeat envoyé");
            }
            Ok(Err(err)) => {
                tracing::warn!(server_id = %self.server_id, "Dernier heartbeat en échec : {err}");
            }
            Err(_) => {
                tracing::warn!(server_id = %self.server_id, "Dernier heartbeat expiré");
            }
        }
    }

    async fn observe_server(&self, server: &Server) {
        let Ok(players) = server.get_players().await else {
            return;
        };
        let max_players = server
            .get_state()
            .await
            .map_or(DEFAULT_MAX_PLAYERS, |state| state.max_players);
        self.observe(max_players, players.iter().map(|p| p.uuid).collect());
    }

    pub fn spawn_loop(self: std::sync::Arc<Self>, server: Server) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
            interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
            let mut failures = FailureLog::default();
            loop {
                interval.tick().await;
                if self.ending.load(Ordering::SeqCst) {
                    break;
                }
                self.observe_server(&server).await;
                match self.beat(ServerPhase::Waiting).await {
                    Ok(()) => {
                        if failures.on_success() {
                            tracing::info!(server_id = %self.server_id, "Heartbeat orchestrateur rétabli");
                        }
                    }
                    Err(err) => {
                        if failures.on_failure(Instant::now()) {
                            tracing::warn!(
                                server_id = %self.server_id,
                                "Échec envoi heartbeat orchestrateur : {err}"
                            );
                        }
                    }
                }
            }
        });
    }
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|raw| raw.trim().to_owned())
        .filter(|val| !val.is_empty())
}
