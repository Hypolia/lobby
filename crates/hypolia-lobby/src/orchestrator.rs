use std::sync::RwLock;
use std::time::Duration;

use hypolia_progression::{Coins, Level, Unlock};
use hypolia_protocol::{
    AdminCoinGrant, CosmeticId, CosmeticSlot, EquipRequest, FleetStatusSnapshot, GameFleetStatus,
    GameKind, HeartbeatRequest, JoinQueueRequest, JoinQueueResponse, LeaveQueueRequest,
    LeaveQueueResponse, LeaveReason, PlayerIdentityUpdate, PlayerProfile, PlayerQueueState,
    PlayerQueueStatus, PlayerRank, PurchaseRequest, SpectateRequest,
};
use serde::Deserialize;
use thiserror::Error;
use tokio::time::Instant;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum OrchestratorClientError {
    #[error("orchestrateur injoignable : {0}")]
    Transport(#[from] reqwest::Error),
    #[error("réponse orchestrateur HTTP {0}")]
    HttpStatus(u16),
    #[error("serveur introuvable")]
    ServerNotFound,
    #[error("serveur non joignable pour le moment")]
    ServerNotJoinable,
}

#[derive(Debug, Error)]
pub enum ProgressionClientError {
    #[error("orchestrateur injoignable : {0}")]
    Transport(#[from] reqwest::Error),
    #[error("progression indisponible")]
    Unavailable,
    #[error("il manque {missing} coins")]
    InsufficientCoins { missing: u64 },
    #[error("grade requis : {0:?}")]
    RankRequired(PlayerRank),
    #[error("level réseau requis : {0}")]
    LevelRequired(u32),
    #[error("déjà débloqué")]
    AlreadyUnlocked,
    #[error("cosmétique inconnu")]
    UnknownCosmetic,
    #[error("non débloqué : {requires:?}")]
    NotUnlocked { requires: Unlock },
    #[error("mauvais emplacement")]
    WrongSlot,
    #[error("réponse orchestrateur HTTP {0}")]
    HttpStatus(u16),
}

#[derive(Deserialize)]
struct ProgressionErrorBody {
    error: String,
    missing: Option<u64>,
    rank: Option<PlayerRank>,
    level: Option<u32>,
    requires: Option<RequiresBody>,
}

#[derive(Deserialize)]
struct RequiresBody {
    price: Option<u64>,
    rank: Option<PlayerRank>,
    network_level: Option<u32>,
}

impl RequiresBody {
    fn into_unlock(self) -> Unlock {
        if let Some(price) = self.price {
            Unlock::Price(Coins(price))
        } else if let Some(rank) = self.rank {
            Unlock::Rank(rank)
        } else if let Some(level) = self.network_level.and_then(Level::new) {
            Unlock::NetworkLevel(level)
        } else {
            Unlock::Free
        }
    }
}

pub fn decode_progression_error(status: u16, body: &[u8]) -> ProgressionClientError {
    let Ok(parsed) = serde_json::from_slice::<ProgressionErrorBody>(body) else {
        return ProgressionClientError::HttpStatus(status);
    };
    match (parsed.error.as_str(), status) {
        ("progression_unavailable", _) => ProgressionClientError::Unavailable,
        ("insufficient_coins", 402) => ProgressionClientError::InsufficientCoins {
            missing: parsed.missing.unwrap_or(0),
        },
        ("rank_required", 403) => parsed
            .rank
            .map_or(ProgressionClientError::HttpStatus(status), |rank| {
                ProgressionClientError::RankRequired(rank)
            }),
        ("level_required", 403) => parsed
            .level
            .map_or(ProgressionClientError::HttpStatus(status), |level| {
                ProgressionClientError::LevelRequired(level)
            }),
        ("already_unlocked", 409) => ProgressionClientError::AlreadyUnlocked,
        ("unknown_cosmetic", 422) => ProgressionClientError::UnknownCosmetic,
        ("not_unlocked", 403) => ProgressionClientError::NotUnlocked {
            requires: parsed
                .requires
                .map_or(Unlock::Free, RequiresBody::into_unlock),
        },
        ("wrong_slot", 422) => ProgressionClientError::WrongSlot,
        _ => ProgressionClientError::HttpStatus(status),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub budget: Duration,
    pub initial: Duration,
    pub max: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            budget: Duration::from_secs(10),
            initial: Duration::from_millis(250),
            max: Duration::from_secs(2),
        }
    }
}

pub struct OrchestratorClient {
    base_url: String,
    token: Option<String>,
    http: reqwest::Client,
    retry: RetryPolicy,
    last_snapshot: RwLock<Option<FleetStatusSnapshot>>,
}

impl OrchestratorClient {
    pub fn new(base_url: String, token: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(4))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            token,
            http,
            retry: RetryPolicy::default(),
            last_snapshot: RwLock::new(None),
        }
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    async fn send_retrying<B>(
        &self,
        build: B,
        deadline: Instant,
    ) -> Result<reqwest::Response, reqwest::Error>
    where
        B: Fn() -> reqwest::RequestBuilder,
    {
        let mut delay = self.retry.initial;
        loop {
            let outcome = build().send().await;
            let transient = match &outcome {
                Ok(resp) => resp.status().is_server_error(),
                Err(_) => true,
            };
            if !transient || Instant::now() + delay > deadline {
                return outcome;
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(self.retry.max);
        }
    }

    pub async fn send_heartbeat(
        &self,
        req: &HeartbeatRequest,
    ) -> Result<(), OrchestratorClientError> {
        let resp = self
            .apply_auth(
                self.http
                    .post(format!("{}/api/v1/servers/heartbeat", self.base_url)),
            )
            .json(req)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(resp.status().as_u16()));
        }
        Ok(())
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(tok) => req.bearer_auth(tok),
            None => req,
        }
    }

    pub async fn fetch_status(&self) -> Result<FleetStatusSnapshot, OrchestratorClientError> {
        let req = self.apply_auth(self.http.get(format!("{}/api/v1/status", self.base_url)));
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(resp.status().as_u16()));
        }
        let snap: FleetStatusSnapshot = resp.json().await?;
        *self
            .last_snapshot
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(snap.clone());
        Ok(snap)
    }

    async fn queue_status(
        &self,
        player_uuid: Uuid,
        game: GameKind,
        deadline: Instant,
    ) -> Result<PlayerQueueStatus, OrchestratorClientError> {
        let status_resp = self
            .send_retrying(
                || {
                    self.apply_auth(
                        self.http
                            .get(format!(
                                "{}/api/v1/queue/player/{player_uuid}",
                                self.base_url
                            ))
                            .query(&[("game", game)]),
                    )
                },
                deadline,
            )
            .await?;
        if !status_resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(
                status_resp.status().as_u16(),
            ));
        }
        let current: PlayerQueueStatus = status_resp.json().await?;
        Ok(current)
    }

    pub async fn is_queued(
        &self,
        player_uuid: Uuid,
        game: GameKind,
    ) -> Result<bool, OrchestratorClientError> {
        let deadline = Instant::now() + self.retry.budget;
        let status = self.queue_status(player_uuid, game, deadline).await?;
        Ok(status.state == PlayerQueueState::Queued)
    }

    pub async fn toggle_queue(
        &self,
        player_uuid: Uuid,
        player_name: &str,
        game: GameKind,
        match_size: u8,
    ) -> Result<ToggleQueueOutcome, OrchestratorClientError> {
        let deadline = Instant::now() + self.retry.budget;
        let current = self.queue_status(player_uuid, game, deadline).await?;
        if current.state == PlayerQueueState::Queued {
            let body = leave_body(player_uuid, LeaveReason::UserToggle, game);
            let resp = self.post_queue("leave", &body, deadline).await?;
            let left: LeaveQueueResponse = resp.json().await?;
            return Ok(ToggleQueueOutcome::Left(left));
        }

        let body = join_body(player_uuid, player_name, game, match_size);
        let resp = self.post_queue("join", &body, deadline).await?;
        let joined: JoinQueueResponse = resp.json().await?;
        Ok(ToggleQueueOutcome::Joined(joined))
    }

    async fn post_queue<T: serde::Serialize + Sync>(
        &self,
        action: &str,
        body: &T,
        deadline: Instant,
    ) -> Result<reqwest::Response, OrchestratorClientError> {
        let resp = self
            .send_retrying(
                || {
                    self.apply_auth(
                        self.http
                            .post(format!("{}/api/v1/queue/{action}", self.base_url)),
                    )
                    .json(body)
                },
                deadline,
            )
            .await?;
        if !resp.status().is_success() {
            return Err(OrchestratorClientError::HttpStatus(resp.status().as_u16()));
        }
        Ok(resp)
    }

    pub async fn leave_if_queued(&self, player_uuid: Uuid) -> Result<(), OrchestratorClientError> {
        let deadline = Instant::now() + self.retry.budget;
        let mut first_error = None;
        for game in GameKind::ALL {
            let body = leave_body(player_uuid, LeaveReason::LobbyQuit, game);
            if let Err(err) = self.post_queue("leave", &body, deadline).await {
                first_error.get_or_insert(err);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub async fn spectate(
        &self,
        server_id: &str,
        player_uuid: Uuid,
    ) -> Result<(), OrchestratorClientError> {
        let req = self
            .apply_auth(self.http.post(format!(
                "{}/api/v1/servers/{server_id}/spectate",
                self.base_url
            )))
            .json(&SpectateRequest { player_uuid });
        let resp = req.send().await?;
        match resp.status().as_u16() {
            404 => Err(OrchestratorClientError::ServerNotFound),
            409 => Err(OrchestratorClientError::ServerNotJoinable),
            code if !resp.status().is_success() => Err(OrchestratorClientError::HttpStatus(code)),
            _ => Ok(()),
        }
    }

    async fn send_for_profile(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<PlayerProfile, ProgressionClientError> {
        let resp = self.apply_auth(req).send().await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp.json().await?);
        }
        let body = resp.bytes().await.unwrap_or_default();
        Err(decode_progression_error(status.as_u16(), &body))
    }

    pub async fn fetch_profile(
        &self,
        player_uuid: Uuid,
    ) -> Result<PlayerProfile, ProgressionClientError> {
        let req = self.http.get(format!(
            "{}/api/v1/players/{player_uuid}/profile",
            self.base_url
        ));
        self.send_for_profile(req).await
    }

    pub async fn post_identity(
        &self,
        player_uuid: Uuid,
        identity: &PlayerIdentityUpdate,
    ) -> Result<(), ProgressionClientError> {
        let req = self
            .http
            .post(format!(
                "{}/api/v1/players/{player_uuid}/identity",
                self.base_url
            ))
            .json(identity);
        let resp = self.apply_auth(req).send().await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(());
        }
        let body = resp.bytes().await.unwrap_or_default();
        Err(decode_progression_error(status.as_u16(), &body))
    }

    pub async fn purchase(
        &self,
        player_uuid: Uuid,
        cosmetic: CosmeticId,
    ) -> Result<PlayerProfile, ProgressionClientError> {
        let req = self
            .http
            .post(format!(
                "{}/api/v1/players/{player_uuid}/purchases",
                self.base_url
            ))
            .json(&PurchaseRequest { cosmetic });
        self.send_for_profile(req).await
    }

    pub async fn equip(
        &self,
        player_uuid: Uuid,
        slot: CosmeticSlot,
        cosmetic: Option<CosmeticId>,
    ) -> Result<PlayerProfile, ProgressionClientError> {
        let req = self
            .http
            .put(format!(
                "{}/api/v1/players/{player_uuid}/equipped",
                self.base_url
            ))
            .json(&EquipRequest { slot, cosmetic });
        self.send_for_profile(req).await
    }

    pub async fn grant_coins(
        &self,
        player_uuid: Uuid,
        grant: &AdminCoinGrant,
    ) -> Result<PlayerProfile, ProgressionClientError> {
        let req = self
            .http
            .post(format!(
                "{}/api/v1/players/{player_uuid}/coins",
                self.base_url
            ))
            .json(grant);
        self.send_for_profile(req).await
    }

    pub fn cached_snapshot(&self) -> Option<FleetStatusSnapshot> {
        self.last_snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn last_snapshot(&self) -> Option<FleetStatusSnapshot> {
        self.last_snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn cached_queue_counts(&self) -> QueueCounts {
        self.last_snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map_or_else(QueueCounts::default, QueueCounts::from_snapshot)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueCounts {
    pub sheepwars: usize,
    pub towerwars: usize,
    /// Players on SheepWars servers right now.
    pub sheepwars_playing: u32,
    /// Players on TowerWars servers right now.
    pub towerwars_playing: u32,
}

impl QueueCounts {
    pub fn from_snapshot(snapshot: &FleetStatusSnapshot) -> Self {
        let per_game = |game: GameKind| {
            snapshot
                .games
                .iter()
                .find(|status| status.game == game)
                .map(|status| status.queue_len)
        };
        let playing = |game: GameKind| {
            snapshot
                .servers
                .iter()
                .filter(|server| server.kind == game.server_kind())
                .map(|server| server.player_count)
                .sum()
        };
        Self {
            sheepwars: per_game(GameKind::SheepWars).unwrap_or(snapshot.queue_len),
            towerwars: per_game(GameKind::TowerWars).unwrap_or(0),
            sheepwars_playing: playing(GameKind::SheepWars),
            towerwars_playing: playing(GameKind::TowerWars),
        }
    }

    pub const fn playing(self, game: GameKind) -> u32 {
        match game {
            GameKind::SheepWars => self.sheepwars_playing,
            GameKind::TowerWars => self.towerwars_playing,
        }
    }

    pub const fn get(self, game: GameKind) -> usize {
        match game {
            GameKind::SheepWars => self.sheepwars,
            GameKind::TowerWars => self.towerwars,
        }
    }

    pub const fn with(mut self, game: GameKind, count: usize) -> Self {
        match game {
            GameKind::SheepWars => self.sheepwars = count,
            GameKind::TowerWars => self.towerwars = count,
        }
        self
    }
}

pub fn game_fleet(snapshot: &FleetStatusSnapshot, game: GameKind) -> Option<GameFleetStatus> {
    if let Some(found) = snapshot.games.iter().find(|status| status.game == game) {
        return Some(found.clone());
    }
    (game == GameKind::SheepWars).then(|| GameFleetStatus {
        game,
        queue_len: snapshot.queue_len,
        min_players: snapshot.min_players,
        max_players_per_server: snapshot.max_players_per_server,
        min_replicas: snapshot.min_replicas,
        max_replicas: snapshot.max_replicas,
        standby_servers: snapshot.standby_servers,
        ready_servers: snapshot.ready_servers,
        in_game_servers: snapshot.in_game_servers,
        starting_servers: snapshot.starting_servers,
        queued_players: snapshot.queued_players.clone(),
    })
}

pub fn join_body(
    player_uuid: Uuid,
    player_name: &str,
    game: GameKind,
    match_size: u8,
) -> JoinQueueRequest {
    JoinQueueRequest {
        player_uuid,
        player_name: player_name.to_owned(),
        game,
        match_size,
    }
}

pub fn leave_body(player_uuid: Uuid, reason: LeaveReason, game: GameKind) -> LeaveQueueRequest {
    LeaveQueueRequest {
        player_uuid,
        reason,
        game,
    }
}

#[derive(Debug, Clone)]
pub enum ToggleQueueOutcome {
    Joined(JoinQueueResponse),
    Left(LeaveQueueResponse),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_bodies_carry_the_game() {
        let uuid = Uuid::nil();
        let join = serde_json::to_value(join_body(uuid, "Alex", GameKind::TowerWars, 3))
            .expect("serialize");
        assert_eq!(join["game"], "tower_wars");
        assert_eq!(join["match_size"], 3);
        let sheep = join_body(uuid, "Alex", GameKind::SheepWars, 0);
        assert_eq!(sheep.match_size, 0);
        let leave = serde_json::to_value(leave_body(
            uuid,
            LeaveReason::UserToggle,
            GameKind::SheepWars,
        ))
        .expect("serialize");
        assert_eq!(leave["game"], "sheep_wars");
    }

    #[test]
    fn queue_counts_prefer_per_game_entries_and_fall_back_to_the_legacy_field() {
        let mut snapshot: FleetStatusSnapshot = serde_json::from_value(serde_json::json!({
            "queue_len": 3, "min_players": 2, "max_players_per_server": 8,
            "min_replicas": 1, "max_replicas": 2, "standby_servers": 0,
            "ready_servers": 0, "in_game_servers": 0, "starting_servers": 0,
            "queued_players": [], "servers": []
        }))
        .expect("snapshot");
        assert_eq!(
            QueueCounts::from_snapshot(&snapshot),
            QueueCounts {
                sheepwars: 3,
                ..QueueCounts::default()
            }
        );
        snapshot.games = vec![GameFleetStatus {
            game: GameKind::TowerWars,
            queue_len: 5,
            min_players: 2,
            max_players_per_server: 8,
            min_replicas: 1,
            max_replicas: 2,
            standby_servers: 0,
            ready_servers: 0,
            in_game_servers: 0,
            starting_servers: 0,
            queued_players: Vec::new(),
        }];
        let counts = QueueCounts::from_snapshot(&snapshot);
        assert_eq!(
            game_fleet(&snapshot, GameKind::TowerWars).map(|g| g.queue_len),
            Some(5)
        );
        assert_eq!(counts.get(GameKind::TowerWars), 5);
        assert_eq!(counts.get(GameKind::SheepWars), 3);
    }

    #[test]
    fn queue_counts_sum_the_players_of_each_game() {
        let snapshot: FleetStatusSnapshot = serde_json::from_value(serde_json::json!({
            "queue_len": 0, "min_players": 2, "max_players_per_server": 8,
            "min_replicas": 1, "max_replicas": 2, "standby_servers": 0,
            "ready_servers": 0, "in_game_servers": 0, "starting_servers": 0,
            "queued_players": [],
            "servers": [
                {"server_id": "sw-1", "address": "a", "phase": "in_game", "player_count": 6,
                 "max_players": 8, "ready": true, "heartbeat_age_secs": 1, "kind": "sheep_wars"},
                {"server_id": "sw-2", "address": "b", "phase": "waiting", "player_count": 2,
                 "max_players": 8, "ready": true, "heartbeat_age_secs": 1, "kind": "sheep_wars"},
                {"server_id": "tw-1", "address": "c", "phase": "in_game", "player_count": 3,
                 "max_players": 4, "ready": true, "heartbeat_age_secs": 1, "kind": "tower_wars"}
            ],
            "lobbies": [
                {"server_id": "lobby-a", "address": "d", "phase": "waiting", "player_count": 9,
                 "max_players": 100, "ready": true, "heartbeat_age_secs": 1, "kind": "lobby"}
            ]
        }))
        .expect("snapshot");
        let counts = QueueCounts::from_snapshot(&snapshot);
        assert_eq!(counts.playing(GameKind::SheepWars), 8);
        assert_eq!(counts.playing(GameKind::TowerWars), 3);
    }

    #[test]
    fn insufficient_coins_carries_the_missing_amount() {
        let err = decode_progression_error(402, br#"{"error":"insufficient_coins","missing":100}"#);
        assert!(matches!(
            err,
            ProgressionClientError::InsufficientCoins { missing: 100 }
        ));
    }

    #[test]
    fn rank_and_level_requirements_are_decoded() {
        let rank = decode_progression_error(403, br#"{"error":"rank_required","rank":"vip"}"#);
        assert!(matches!(
            rank,
            ProgressionClientError::RankRequired(PlayerRank::Vip)
        ));
        let level = decode_progression_error(403, br#"{"error":"level_required","level":10}"#);
        assert!(matches!(level, ProgressionClientError::LevelRequired(10)));
    }

    #[test]
    fn not_unlocked_decodes_every_requirement_shape() {
        let cases: [(&[u8], Unlock); 4] = [
            (
                br#"{"error":"not_unlocked","requires":{"price":500}}"#,
                Unlock::Price(Coins(500)),
            ),
            (
                br#"{"error":"not_unlocked","requires":{"rank":"vip"}}"#,
                Unlock::Rank(PlayerRank::Vip),
            ),
            (
                br#"{"error":"not_unlocked","requires":{"network_level":10}}"#,
                Unlock::NetworkLevel(Level::new(10).expect("non zero")),
            ),
            (
                br#"{"error":"not_unlocked","requires":{"free":null}}"#,
                Unlock::Free,
            ),
        ];
        for (body, expected) in cases {
            match decode_progression_error(403, body) {
                ProgressionClientError::NotUnlocked { requires } => assert_eq!(requires, expected),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn simple_errors_and_unavailability_are_decoded() {
        assert!(matches!(
            decode_progression_error(409, br#"{"error":"already_unlocked"}"#),
            ProgressionClientError::AlreadyUnlocked
        ));
        assert!(matches!(
            decode_progression_error(422, br#"{"error":"unknown_cosmetic"}"#),
            ProgressionClientError::UnknownCosmetic
        ));
        assert!(matches!(
            decode_progression_error(422, br#"{"error":"wrong_slot"}"#),
            ProgressionClientError::WrongSlot
        ));
        assert!(matches!(
            decode_progression_error(503, br#"{"error":"progression_unavailable"}"#),
            ProgressionClientError::Unavailable
        ));
        assert!(matches!(
            decode_progression_error(500, b"oops"),
            ProgressionClientError::HttpStatus(500)
        ));
    }
}
