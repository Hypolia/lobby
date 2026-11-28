use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use rspigot_plugin::{Facing, Material, Player, Position, Rotation};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::storage::{
    Backoff, FileLayoutRepository, LayoutBackend, LayoutRepository, RepositoryError, WriteGate,
    load_or_seed,
};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("{0}")]
    Repository(#[from] RepositoryError),
    #[error("indice hors limites : {0}")]
    OutOfBounds(String),
}

#[derive(Debug, Clone)]
pub struct EnvConfig {
    pub ferriskey_base_url: String,
    pub ferriskey_public_url: Option<String>,
    pub ferriskey_realm: String,
    pub ferriskey_client_id: String,
    pub ferriskey_client_secret: Option<String>,
    pub ferriskey_scope: String,
    pub capitainerie_url: Option<String>,
    pub capitainerie_console_url: Option<String>,
    pub capitainerie_resource: Option<String>,
    pub capitainerie_observer_token: Option<String>,
    pub capitainerie_ferriskey_url: String,
    pub capitainerie_ferriskey_public_url: Option<String>,
    pub capitainerie_realm: String,
    pub capitainerie_client_id: String,
    pub capitainerie_client_secret: Option<String>,
    pub capitainerie_scope: String,
    pub capitainerie_sessions_path: PathBuf,
    pub sheepwars_transfer: Option<(String, u16)>,
    pub sheepwars_backend: Option<String>,
    pub sheepwars_world: Option<String>,
    pub orchestrator_url: Option<String>,
    pub orchestrator_token: Option<String>,
    pub layout_path: PathBuf,
    pub sessions_path: PathBuf,
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|raw| raw.trim().to_owned())
        .filter(|val| !val.is_empty())
}

pub fn lobby_version() -> Option<String> {
    non_empty_env("HYPOLIA_VERSION")
}

impl EnvConfig {
    pub fn from_env() -> Self {
        let ferriskey_base_url = non_empty_env("FERRISKEY_URL")
            .unwrap_or_else(|| "http://127.0.0.1:3333".to_owned())
            .trim_end_matches('/')
            .to_owned();
        let ferriskey_public_url =
            non_empty_env("FERRISKEY_PUBLIC_URL").map(|url| url.trim_end_matches('/').to_owned());
        let ferriskey_realm =
            non_empty_env("FERRISKEY_REALM").unwrap_or_else(|| "capitainerie".to_owned());
        let ferriskey_client_id =
            non_empty_env("FERRISKEY_CLIENT_ID").unwrap_or_else(|| "minecraft".to_owned());
        let ferriskey_client_secret = non_empty_env("FERRISKEY_CLIENT_SECRET");
        let ferriskey_scope = non_empty_env("FERRISKEY_SCOPE")
            .unwrap_or_else(|| "openid profile email roles".to_owned());

        let capitainerie_url =
            non_empty_env("CAPITAINERIE_URL").map(|url| url.trim_end_matches('/').to_owned());
        let capitainerie_console_url = non_empty_env("CAPITAINERIE_CONSOLE_URL")
            .map(|url| url.trim_end_matches('/').to_owned());
        let capitainerie_resource =
            non_empty_env("CAPITAINERIE_RESOURCE").or_else(|| capitainerie_url.clone());
        let capitainerie_observer_token = non_empty_env("CAPITAINERIE_OBSERVER_TOKEN");

        let capitainerie_ferriskey_url = non_empty_env("CAPITAINERIE_FERRISKEY_URL")
            .unwrap_or_else(|| ferriskey_base_url.clone())
            .trim_end_matches('/')
            .to_owned();
        let capitainerie_ferriskey_public_url = non_empty_env("CAPITAINERIE_FERRISKEY_PUBLIC_URL")
            .map(|url| url.trim_end_matches('/').to_owned())
            .or_else(|| {
                if capitainerie_ferriskey_url == ferriskey_base_url {
                    ferriskey_public_url.clone()
                } else {
                    None
                }
            });
        let capitainerie_realm =
            non_empty_env("CAPITAINERIE_REALM").unwrap_or_else(|| ferriskey_realm.clone());
        let capitainerie_client_id =
            non_empty_env("CAPITAINERIE_CLIENT_ID").unwrap_or_else(|| ferriskey_client_id.clone());
        let same_auth_client = capitainerie_ferriskey_url == ferriskey_base_url
            && capitainerie_realm == ferriskey_realm
            && capitainerie_client_id == ferriskey_client_id;
        let capitainerie_client_secret =
            non_empty_env("CAPITAINERIE_CLIENT_SECRET").or_else(|| {
                if same_auth_client {
                    ferriskey_client_secret.clone()
                } else {
                    None
                }
            });
        let capitainerie_scope =
            non_empty_env("CAPITAINERIE_SCOPE").unwrap_or_else(|| ferriskey_scope.clone());
        let capitainerie_sessions_path = non_empty_env("CAPITAINERIE_SESSIONS_PATH").map_or_else(
            || PathBuf::from(".capitainerie-sessions.json"),
            PathBuf::from,
        );

        let sheepwars_transfer = non_empty_env("SHEEPWARS_HOST").map(|host| {
            let port = non_empty_env("SHEEPWARS_PORT")
                .and_then(|val| val.parse::<u16>().ok())
                .unwrap_or(25565);
            (host, port)
        });
        let sheepwars_backend = non_empty_env("SHEEPWARS_BACKEND");
        let sheepwars_world = non_empty_env("SHEEPWARS_WORLD");
        let orchestrator_url =
            non_empty_env("ORCHESTRATOR_URL").map(|url| url.trim_end_matches('/').to_owned());
        let orchestrator_token = non_empty_env("ORCHESTRATOR_TOKEN");

        let layout_path = non_empty_env("LOBBY_LAYOUT_PATH")
            .map_or_else(|| PathBuf::from("lobby.json"), PathBuf::from);
        let sessions_path = non_empty_env("LOBBY_SESSIONS_PATH")
            .map_or_else(|| PathBuf::from(".sessions.json"), PathBuf::from);

        Self {
            ferriskey_base_url,
            ferriskey_public_url,
            ferriskey_realm,
            ferriskey_client_id,
            ferriskey_client_secret,
            ferriskey_scope,
            capitainerie_url,
            capitainerie_console_url,
            capitainerie_resource,
            capitainerie_observer_token,
            capitainerie_ferriskey_url,
            capitainerie_ferriskey_public_url,
            capitainerie_realm,
            capitainerie_client_id,
            capitainerie_client_secret,
            capitainerie_scope,
            capitainerie_sessions_path,
            sheepwars_transfer,
            sheepwars_backend,
            sheepwars_world,
            orchestrator_url,
            orchestrator_token,
            layout_path,
            sessions_path,
        }
    }

    pub fn has_dedicated_capitainerie_auth(&self) -> bool {
        self.capitainerie_ferriskey_url != self.ferriskey_base_url
            || self.capitainerie_realm != self.ferriskey_realm
            || self.capitainerie_client_id != self.ferriskey_client_id
    }

    pub fn device_auth_url(&self) -> String {
        format!(
            "{}/realms/{}/protocol/openid-connect/auth/device",
            self.ferriskey_base_url, self.ferriskey_realm
        )
    }

    pub fn token_url(&self) -> String {
        format!(
            "{}/realms/{}/protocol/openid-connect/token",
            self.ferriskey_base_url, self.ferriskey_realm
        )
    }

    pub fn capitainerie_device_auth_url(&self) -> String {
        format!(
            "{}/realms/{}/protocol/openid-connect/auth/device",
            self.capitainerie_ferriskey_url, self.capitainerie_realm
        )
    }

    pub fn capitainerie_token_url(&self) -> String {
        format!(
            "{}/realms/{}/protocol/openid-connect/token",
            self.capitainerie_ferriskey_url, self.capitainerie_realm
        )
    }

    pub fn console_task_url(&self, card_id: &str) -> Option<String> {
        if card_id.is_empty() {
            return None;
        }
        self.capitainerie_console_url
            .as_deref()
            .map(|base| format!("{base}/tasks/{card_id}"))
    }
}

/// Bottom of the board: one block above the lobby floor at (27, -85).
pub const LEADERBOARD_SPOT: WaypointPose = WaypointPose::new(27.5, 30.0, -84.5, 0.0, 0.0);

/// First bundled position, three blocks above the floor and too high. A stored
/// layout still holding it was never moved by an admin and is migrated.
pub const LEADERBOARD_FIRST_SPOT: WaypointPose = WaypointPose::new(27.5, 32.0, -84.5, 0.0, 0.0);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WaypointPose {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    #[serde(default)]
    pub yaw: f32,
    #[serde(default)]
    pub pitch: f32,
}

impl WaypointPose {
    pub const fn new(x: f64, y: f64, z: f64, yaw: f32, pitch: f32) -> Self {
        Self {
            x,
            y,
            z,
            yaw,
            pitch,
        }
    }

    pub fn from_player(player: &Player) -> Self {
        Self {
            x: (player.position.x * 100.0).round() / 100.0,
            y: (player.position.y * 100.0).round() / 100.0,
            z: (player.position.z * 100.0).round() / 100.0,
            yaw: (player.rotation.yaw * 10.0).round() / 10.0,
            pitch: (player.rotation.pitch * 10.0).round() / 10.0,
        }
    }

    pub const fn position(self) -> Position {
        Position {
            x: self.x,
            y: self.y,
            z: self.z,
        }
    }

    pub const fn rotation(self) -> Rotation {
        Rotation {
            yaw: self.yaw,
            pitch: self.pitch,
        }
    }

    pub const fn facing(self) -> Facing {
        Facing::new(self.position(), self.rotation())
    }

    pub fn project_forward(self, distance: f64) -> Position {
        let rad = self.yaw.to_radians();
        Position {
            x: self.x - f64::from(rad.sin()) * distance,
            y: self.y,
            z: self.z + f64::from(rad.cos()) * distance,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PedestalSpec {
    pub index: u8,
    pub id: String,
    pub npc_name: String,
    pub title: String,
    pub subtitle: String,
    pub game_id: String,
    pub active: bool,
    pub main_hand: String,
    #[serde(default)]
    pub off_hand: Option<String>,
    pub pose: WaypointPose,
}

impl PedestalSpec {
    pub fn main_material(&self) -> Option<Material> {
        Material::from_key(&self.main_hand)
    }

    pub fn off_material(&self) -> Option<Material> {
        self.off_hand.as_deref().and_then(Material::from_key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmbientRole {
    Guard,
    Lumberjack,
    PortalMage,
    Innkeeper,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AmbientNpcSpec {
    pub id: String,
    pub npc_name: String,
    pub display_name: String,
    pub subtitle: String,
    pub role: AmbientRole,
    pub walk_speed: f64,
    pub waypoints: Vec<WaypointPose>,
    pub dialogues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LobbyLayout {
    pub spawn: WaypointPose,
    pub void_y: f64,
    pub pois: BTreeMap<String, WaypointPose>,
    pub pedestals: Vec<PedestalSpec>,
    pub ambient_npcs: Vec<AmbientNpcSpec>,
    pub captain: WaypointPose,
    pub agent_idle_origin: WaypointPose,
    pub agent_idle_step_x: f64,
    pub agent_workstations: Vec<WaypointPose>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leaderboard: Option<WaypointPose>,
}

impl Default for LobbyLayout {
    fn default() -> Self {
        // Parvis d'arrivée en haut du tapis rouge : bloc (13, 29, -83) -> (13.5, 29.0, -82.5), vue SUD (+Z, yaw = 0.0)
        let spawn = WaypointPose::new(13.5, 29.0, -82.5, 0.0, 0.0);
        let mut pois = BTreeMap::new();
        pois.insert("spawn".to_owned(), spawn);
        pois.insert(
            "plaza".to_owned(),
            WaypointPose::new(13.5, 26.0, -64.5, 0.0, 0.0),
        );
        pois.insert(
            "portal".to_owned(),
            WaypointPose::new(13.5, 22.0, -22.5, 0.0, 8.0),
        );
        pois.insert(
            "capitainerie".to_owned(),
            WaypointPose::new(24.5, 24.0, -35.5, -90.0, 0.0),
        );
        pois.insert(
            "castle".to_owned(),
            WaypointPose::new(-49.5, 62.0, 60.5, -145.0, 12.0),
        );

        // Coordonnées exactes des 2 socles white_concrete (Y=26 -> pose Y=27.0) de la Grande Galerie au tapis rouge
        let pedestals = vec![
            PedestalSpec {
                index: 4,
                id: "sheepwars".to_owned(),
                npc_name: "SheepWars".to_owned(),
                title: "§d§lSHEEP WARS".to_owned(),
                subtitle: "§a▶ Clic droit pour jouer !".to_owned(),
                game_id: "sheepwars".to_owned(),
                active: true,
                main_hand: "minecraft:white_wool".to_owned(),
                off_hand: Some("minecraft:bow".to_owned()),
                pose: WaypointPose::new(13.5, 27.0, -48.5, 180.0, 0.0),
            },
            PedestalSpec {
                index: 7,
                id: "towerwars".to_owned(),
                npc_name: "TowerWars".to_owned(),
                title: "§b§lTOWER WARS".to_owned(),
                subtitle: "§a▶ Clic droit pour jouer !".to_owned(),
                game_id: "towerwars".to_owned(),
                active: true,
                main_hand: "minecraft:crossbow".to_owned(),
                off_hand: Some("minecraft:arrow".to_owned()),
                pose: WaypointPose::new(0.5, 27.0, -70.5, -105.0, 0.0),
            },
        ];

        let ambient_npcs = vec![
            AmbientNpcSpec {
                id: "guard".to_owned(),
                npc_name: "Garde_Pont".to_owned(),
                display_name: "§9§lGarde Halvard".to_owned(),
                subtitle: "§7Sentinelle de la Citadelle".to_owned(),
                role: AmbientRole::Guard,
                walk_speed: 3.1,
                waypoints: vec![
                    WaypointPose::new(13.5, 22.0, -39.5, 0.0, 0.0),
                    WaypointPose::new(16.5, 22.0, -34.5, 35.0, 0.0),
                    WaypointPose::new(13.5, 22.0, -29.5, 180.0, 0.0),
                    WaypointPose::new(10.5, 22.0, -34.5, -145.0, 0.0),
                ],
                dialogues: vec![
                    "Bienvenue dans la Citadelle Hypolia ! Le socle central de la galerie mène à SheepWars."
                        .to_owned(),
                    "Votre identité FerrisKey débloque vos grades et l'accès au Capitaine sur l'esplanade Est !"
                        .to_owned(),
                    "Avancez jusqu'à la Place Centrale pour admirer le Vortex Pourpre et les dirigeables."
                        .to_owned(),
                ],
            },
            AmbientNpcSpec {
                id: "lumberjack".to_owned(),
                npc_name: "Bucheron_Olaf".to_owned(),
                display_name: "§2§lOlaf le Charpentier".to_owned(),
                subtitle: "§7Charpentes & Dirigeables".to_owned(),
                role: AmbientRole::Lumberjack,
                walk_speed: 2.8,
                waypoints: vec![
                    WaypointPose::new(-5.5, 24.0, -30.5, 0.0, 0.0),
                    WaypointPose::new(-3.5, 24.0, -22.5, 90.0, 0.0),
                    WaypointPose::new(-6.5, 24.0, -15.5, 180.0, 0.0),
                ],
                dialogues: vec![
                    "Tout ce bois d'épicéa sert à caréner les dirigeables amarrés au-dessus du château !"
                        .to_owned(),
                    "Attention aux moutons explosifs de SheepWars, ils soufflent une coque en deux secondes."
                        .to_owned(),
                ],
            },
            AmbientNpcSpec {
                id: "mage".to_owned(),
                npc_name: "Mage_Astrid".to_owned(),
                display_name: "§5§lMage Astrid".to_owned(),
                subtitle: "§7Gardienne du Vortex".to_owned(),
                role: AmbientRole::PortalMage,
                walk_speed: 2.4,
                waypoints: vec![
                    WaypointPose::new(9.5, 22.0, -24.5, -35.0, 8.0),
                    WaypointPose::new(13.5, 22.0, -24.5, 0.0, 12.0),
                    WaypointPose::new(17.5, 22.0, -24.5, 35.0, 8.0),
                ],
                dialogues: vec![
                    "Le Vortex Pourpre au centre de la place canalise l'énergie vers les arènes SheepWars."
                        .to_owned(),
                    "Sautez dans le Vortex, cliquez sur le socle SheepWars ou utilisez votre Boussole pour jouer !"
                        .to_owned(),
                ],
            },
            AmbientNpcSpec {
                id: "innkeeper".to_owned(),
                npc_name: "Aubergiste_Freya".to_owned(),
                display_name: "§6§lFreya la Marchande".to_owned(),
                subtitle: "§7Allée des Échoppes".to_owned(),
                role: AmbientRole::Innkeeper,
                walk_speed: 2.6,
                waypoints: vec![
                    WaypointPose::new(26.5, 24.0, -23.5, -90.0, 0.0),
                    WaypointPose::new(29.5, 24.0, -17.5, -90.0, 0.0),
                    WaypointPose::new(26.5, 24.0, -11.5, -135.0, 0.0),
                ],
                dialogues: vec![
                    "Bienvenue sur la Grand-Place ! Réglez votre vitesse ou activez le vol dans les Options."
                        .to_owned(),
                    "Depuis les remparts du château, la vue sur la flotte de dirigeables est imprenable !"
                        .to_owned(),
                ],
            },
        ];

        Self {
            spawn,
            void_y: -10.0,
            pois,
            pedestals,
            ambient_npcs,
            captain: WaypointPose::new(29.5, 24.0, -35.5, 90.0, 0.0),
            agent_idle_origin: WaypointPose::new(27.5, 24.0, -39.5, 0.0, 0.0),
            agent_idle_step_x: 1.8,
            agent_workstations: vec![
                WaypointPose::new(33.5, 24.0, -38.5, -90.0, 12.0),
                WaypointPose::new(33.5, 24.0, -35.5, -90.0, 12.0),
                WaypointPose::new(32.5, 24.0, -31.5, -45.0, 12.0),
                WaypointPose::new(28.5, 24.0, -31.5, 0.0, 12.0),
            ],
            leaderboard: Some(LEADERBOARD_SPOT),
        }
    }
}

pub struct LayoutStore {
    repo: LayoutBackend,
    gate: Arc<WriteGate>,
    inner: RwLock<LobbyLayout>,
    write_order: tokio::sync::Mutex<()>,
}

impl LayoutStore {
    pub async fn open(
        repo: LayoutBackend,
        seed: &LobbyLayout,
        gate: Arc<WriteGate>,
        backoff: Backoff,
    ) -> Result<Self, RepositoryError> {
        let layout = load_or_seed(&repo, seed, backoff).await?;
        Ok(Self {
            repo,
            gate,
            inner: RwLock::new(layout),
            write_order: tokio::sync::Mutex::new(()),
        })
    }

    pub fn in_memory(layout: LobbyLayout) -> Self {
        Self {
            repo: LayoutBackend::File(FileLayoutRepository::new(PathBuf::new())),
            gate: Arc::new(WriteGate::default()),
            inner: RwLock::new(layout),
            write_order: tokio::sync::Mutex::new(()),
        }
    }

    pub fn snapshot(&self) -> LobbyLayout {
        self.inner
            .read()
            .map_or_else(|e| e.into_inner().clone(), |guard| guard.clone())
    }

    pub async fn reload(&self) -> Result<LobbyLayout, ConfigError> {
        let Some(stored) = self.repo.load().await? else {
            return Ok(self.snapshot());
        };
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        *guard = stored.clone();
        Ok(stored)
    }

    pub async fn update<F>(&self, mutate: F) -> Result<LobbyLayout, ConfigError>
    where
        F: FnOnce(&mut LobbyLayout) -> Result<(), ConfigError>,
    {
        let _order = self.write_order.lock().await;
        let _write = self.gate.try_enter()?;
        let mut next = self.snapshot();
        mutate(&mut next)?;
        self.repo.save(&next).await?;
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        guard.clone_from(&next);
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layout_stored_before_the_leaderboard_still_loads_without_one() {
        let mut value = serde_json::to_value(LobbyLayout::default()).expect("json");
        value.as_object_mut().expect("object").remove("leaderboard");
        let layout: LobbyLayout = serde_json::from_value(value).expect("loads");
        assert_eq!(layout.leaderboard, None);
    }

    #[test]
    fn the_leaderboard_position_survives_a_round_trip() {
        let spot = WaypointPose::new(1.5, 30.0, -4.5, 90.0, 0.0);
        let layout = LobbyLayout {
            leaderboard: Some(spot),
            ..LobbyLayout::default()
        };
        let json = serde_json::to_string(&layout).expect("json");
        let back: LobbyLayout = serde_json::from_str(&json).expect("loads");
        assert_eq!(back.leaderboard, Some(spot));
    }

    #[test]
    fn default_layout_has_two_pedestals_with_sheepwars_at_index_4() {
        let layout = LobbyLayout::default();
        assert_eq!(layout.pedestals.len(), 2);
        let center = &layout.pedestals[0];
        assert_eq!(center.index, 4);
        assert_eq!(center.game_id, "sheepwars");
        assert!(center.active);
        assert_eq!(center.main_material(), Some(Material::WhiteWool));
    }

    #[test]
    fn default_layout_has_an_active_towerwars_pedestal_matching_its_game_slug() {
        let layout = LobbyLayout::default();
        let tower = layout
            .pedestals
            .iter()
            .find(|p| p.id == "towerwars")
            .expect("towerwars pedestal");
        assert!(tower.active);
        assert_eq!(
            Some(hypolia_protocol::GameKind::TowerWars),
            hypolia_protocol::GameKind::from_slug(&tower.game_id)
        );
        assert_eq!(tower.main_material(), Some(Material::Crossbow));
        let indexes: std::collections::BTreeSet<_> =
            layout.pedestals.iter().map(|p| p.index).collect();
        assert_eq!(indexes.len(), layout.pedestals.len());
    }

    async fn open_file_store(path: &std::path::Path) -> LayoutStore {
        LayoutStore::open(
            LayoutBackend::File(FileLayoutRepository::new(path.to_path_buf())),
            &LobbyLayout::default(),
            Arc::new(WriteGate::default()),
            Backoff::startup(),
        )
        .await
        .expect("open store")
    }

    #[tokio::test]
    async fn layout_store_updates_and_persists_cleanly() {
        let dir = std::env::temp_dir().join(format!("hypolia_cfg_{}", uuid::Uuid::new_v4()));
        let file = dir.join("lobby.json");
        let store = open_file_store(&file).await;
        assert!(file.exists());

        let new_spawn = WaypointPose::new(12.5, 70.0, -3.5, 90.0, 5.0);
        store
            .update(|layout| {
                layout.spawn = new_spawn;
                layout.pois.insert("spawn".to_owned(), new_spawn);
                Ok(())
            })
            .await
            .expect("update spawn");

        let reloaded = open_file_store(&file).await;
        assert_eq!(reloaded.snapshot().spawn, new_spawn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn closed_gate_rejects_layout_updates() {
        let gate = Arc::new(WriteGate::default());
        let store = LayoutStore::open(
            LayoutBackend::File(FileLayoutRepository::new(PathBuf::new())),
            &LobbyLayout::default(),
            Arc::clone(&gate),
            Backoff::startup(),
        )
        .await
        .expect("open store");
        gate.close().await;
        let err = store.update(|_| Ok(())).await.expect_err("rejected");
        assert!(matches!(
            err,
            ConfigError::Repository(RepositoryError::Unavailable(_))
        ));
    }

    #[test]
    fn workspace_lobby_json_matches_default_and_south_projection_is_positive_z() {
        let root_json = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lobby.json");
        let snap = crate::storage::bundled_layout(&root_json).expect("load workspace lobby.json");
        assert_eq!(snap, LobbyLayout::default());
        assert_eq!(snap.spawn, WaypointPose::new(13.5, 29.0, -82.5, 0.0, 0.0));

        let forward = snap.spawn.project_forward(10.0);
        assert!((forward.x - 13.5).abs() < 1e-6);
        assert!((forward.z - (-72.5)).abs() < 1e-6);
    }

    #[test]
    fn workspace_dotenv_files_parse_all_entries_without_truncating() {
        for rel in ["../../.env", "../../.env.example"] {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
            if rel == "../../.env" && !path.exists() {
                continue;
            }
            let iter = dotenvy::from_path_iter(&path).expect("open env file");
            let mut entries = BTreeMap::new();
            for item in iter {
                let (k, v) = item.unwrap_or_else(|e| panic!("invalid line in {rel}: {e}"));
                entries.insert(k, v);
            }
            assert!(
                entries.contains_key("SHEEPWARS_HOST"),
                "{rel} should parse entries after FERRISKEY_SCOPE"
            );
            if rel == "../../.env" {
                assert_eq!(
                    entries.get("CAPITAINERIE_URL").map(String::as_str),
                    Some("http://localhost:4100")
                );
            }
        }
    }
}
