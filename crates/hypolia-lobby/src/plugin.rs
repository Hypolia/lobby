use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use hypolia_progression::Catalog;
use hypolia_protocol::{
    AdminCoinGrant, CosmeticId, CosmeticSlot, GameKind, PlayerIdentityUpdate, PlayerProfile,
    QueueJoinStatus,
};
use rspigot_plugin::prelude::*;
use rspigot_plugin::{
    Action, BarColor, BarStyle, EquipmentSlot, EventResult, Explosion, Facing, Particle,
    ParticleBurst, SoundCue, Uuid, Vector,
};

use crate::auth::{AuthError, AuthManager, Rank, build_device_prompt, build_device_prompt_for};
use crate::capitainerie::{
    CANCEL_BUTTON_ID, CAP_LOGIN_BUTTON_ID, CAPTAIN_DISPLAY_NAME, LOGIN_BUTTON_ID,
    RESULT_BUTTON_PREFIX, RUN_BUTTON_PREFIX, RunStarted, TaskResultReport, TeamAgent,
    build_run_completion_message, build_run_result_dialog, click_slug, clip_text,
    extract_agent_reply, step_name,
};
use crate::chat::{ChatBadge, ChatSystem};
use crate::config::{ConfigError, EnvConfig, LayoutStore, WaypointPose};
use crate::gadgets::{AuraTrail, GadgetManager, LobbyGadget};
use crate::heartbeat::HeartbeatReporter;
use crate::hud::{HubMetrics, HudManager, PlayerHudInput};
use crate::leaderboard::LeaderboardHud;
use crate::menus::{
    GadgetsMenuAction, GamesMenuAction, MenuManager, OptionsMenuAction, apply_lobby_hotbar,
    decode_gadgets_click, decode_games_click, decode_options_click, decode_towerwars_size_click,
};
use crate::network::network_view;
use crate::npcs::{CaptainTurn, NpcClickTarget, NpcDirector, pedestal_game};
use crate::orchestrator::{
    OrchestratorClient, OrchestratorClientError, QueueCounts, ToggleQueueOutcome, game_fleet,
};
use crate::prefs::PrefsStore;
use crate::profile::{
    ProfileCache, ShopItemState, can_grant_coins, equipped_gadget, equipped_trail, error_message,
    gadget_usable, locked_cosmetic_message, next_usable_trail, requirement_message,
    shop_item_state, to_player_rank, trail_usable,
};
use crate::progress_menus::{
    ProfileMenuAction, ProgressMenuKind, ShopMenuAction, cosmetic_title, decode_profile_click,
    decode_shop_click, game_label,
};
use crate::servers_menu::decode_servers_slot;

const MAX_TASK_TITLE: u32 = 120;
const MAX_TASK_DESC: u32 = 2000;
const HUD_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const PORTAL_DEBOUNCE: Duration = Duration::from_millis(400);
const PORTAL_BLAST_POWER: f32 = 6.0;
const PORTAL_LAUNCH_MIN_SPEED: f64 = 0.9;
const PORTAL_LAUNCH_MAX_SPEED: f64 = 2.0;
const PORTAL_LAUNCH_LIFT: f64 = 1.1;
const PORTAL_LAUNCH_DELAY: Duration = Duration::from_millis(60);
const RUN_WATCH_POLL_INTERVAL: Duration = Duration::from_secs(1);
const RUN_WATCH_MAX_POLLS: usize = 900;
const RUN_FINISH_BAR_LINGER: Duration = Duration::from_secs(3);
const MAX_CACHED_RESULTS: usize = 128;
const LOBBY_SUBCOMMANDS: [&str; 9] = [
    "info",
    "reload",
    "setspawn",
    "setpoi",
    "setpedestal",
    "setleaderboard",
    "setcaptain",
    "setstation",
    "setwaypoint",
];

struct SharedState {
    config: EnvConfig,
    layout: Arc<LayoutStore>,
    auth: Arc<AuthManager>,
    prefs: PrefsStore,
    profiles: ProfileCache,
    menus: MenuManager,
    gadgets: GadgetManager,
    hud: HudManager,
    chat: ChatSystem,
    npcs: Arc<NpcDirector>,
    leaderboard: Arc<LeaderboardHud>,
    orchestrator: Option<Arc<OrchestratorClient>>,
    heartbeat: Option<Arc<HeartbeatReporter>>,
    lobby_version: Option<String>,
    sheepwars_queue: RwLock<HashSet<Uuid>>,
    portal_last_blast: RwLock<HashMap<Uuid, Instant>>,
    last_steps: RwLock<HashMap<Uuid, Vector>>,
    task_results: RwLock<HashMap<String, TaskResultReport>>,
}

#[derive(Plugin, Clone)]
pub struct HypoliaLobby {
    state: Arc<SharedState>,
}

impl HypoliaLobby {
    pub fn new(
        config: EnvConfig,
        layout: LayoutStore,
        auth: AuthManager,
        heartbeat: Option<Arc<HeartbeatReporter>>,
    ) -> Self {
        let layout = Arc::new(layout);
        let auth = Arc::new(auth);
        let npcs = Arc::new(NpcDirector::new(
            Arc::clone(&layout),
            config.capitainerie_url.clone(),
        ));
        let orchestrator = config.orchestrator_url.clone().map(|url| {
            Arc::new(OrchestratorClient::new(
                url,
                config.orchestrator_token.clone(),
            ))
        });
        let leaderboard = Arc::new(LeaderboardHud::new(
            Arc::clone(&layout),
            orchestrator.clone(),
        ));
        Self {
            state: Arc::new(SharedState {
                config,
                layout,
                auth,
                prefs: PrefsStore::default(),
                profiles: ProfileCache::default(),
                menus: MenuManager::default(),
                gadgets: GadgetManager::default(),
                hud: HudManager::default(),
                chat: ChatSystem::default(),
                npcs,
                leaderboard,
                orchestrator,
                heartbeat,
                lobby_version: crate::config::lobby_version(),
                sheepwars_queue: RwLock::new(HashSet::new()),
                portal_last_blast: RwLock::new(HashMap::new()),
                last_steps: RwLock::new(HashMap::new()),
                task_results: RwLock::new(HashMap::new()),
            }),
        }
    }

    fn queued_counts(&self) -> QueueCounts {
        if let Some(client) = &self.state.orchestrator {
            return client.cached_queue_counts();
        }
        let sheepwars = self
            .state
            .sheepwars_queue
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len();
        QueueCounts::default().with(GameKind::SheepWars, sheepwars)
    }

    async fn collect_metrics(&self, server: &Server) -> HubMetrics {
        let (local_online, tps_1m) = match server.get_state().await {
            Ok(st) => (st.online, st.tps[0]),
            Err(_) => (0, 20.0),
        };
        let snapshot = self
            .state
            .orchestrator
            .as_ref()
            .and_then(|orch| orch.last_snapshot());
        let own_id = self.state.heartbeat.as_ref().map(|hb| hb.server_id());
        let network = network_view(snapshot.as_ref(), own_id, local_online);
        let (agents_working, agents_total) = self.state.npcs.agent_counts();
        HubMetrics {
            online: network.online,
            in_game: network.in_game,
            lobby_number: network.lobby_number,
            tps_1m,
            agents_working,
            agents_total,
        }
    }

    async fn refresh_single_player_ui(&self, player: &Player) {
        let claims = self.state.auth.claims_of(player.uuid);
        let prefs = self.state.prefs.get(player.uuid);
        apply_lobby_hotbar(player, &prefs);
        self.state.menus.refresh_open_options(player.uuid, &prefs);
        self.state.profiles.with(player.uuid, |profile| {
            self.state
                .menus
                .refresh_open_gadgets(player.uuid, &prefs, profile);
            self.state
                .menus
                .refresh_open_progress(player.uuid, profile, claims.as_ref());
        });

        let (ping_ms, is_op) = match player.get_state().await {
            Ok(st) => (st.ping, st.op),
            Err(_) => (0, false),
        };
        let has_admin = player.permissions.has("hypolia.admin");
        let rank = Rank::resolve(claims.as_ref(), is_op, has_admin);
        let metrics = self.collect_metrics(&player.server()).await;
        self.state.hud.refresh_player(
            player,
            &PlayerHudInput {
                rank,
                ferriskey_username: claims.map(|c| c.username),
                ping_ms,
                metrics,
                network_level: self.state.profiles.level(player.uuid),
                level_percent: self.state.profiles.level_percent(player.uuid),
                coins: self.state.profiles.coins(player.uuid),
            },
        );
    }

    fn store_profile(&self, uuid: Uuid, profile: PlayerProfile, apply_equipped: bool) {
        if apply_equipped {
            let gadget = equipped_gadget(&profile);
            let trail = equipped_trail(&profile);
            if gadget.is_some() || trail.is_some() {
                self.state.prefs.update(uuid, |p| {
                    if let Some(gadget) = gadget {
                        p.equipped_gadget = Some(gadget);
                    }
                    if let Some(trail) = trail {
                        p.aura_trail = trail;
                    }
                });
            }
        }
        self.state.profiles.store(uuid, profile);
    }

    async fn sync_progression(&self, player: &Player) {
        let Some(client) = &self.state.orchestrator else {
            return;
        };
        let claims = self.state.auth.claims_of(player.uuid);
        let is_op = player.get_state().await.is_ok_and(|s| s.op);
        let has_admin = player.permissions.has("hypolia.admin");
        let rank = Rank::resolve(claims.as_ref(), is_op, has_admin);
        let identity = PlayerIdentityUpdate {
            name: player.name.clone(),
            rank: to_player_rank(rank),
        };
        if let Err(err) = client.post_identity(player.uuid, &identity).await {
            tracing::warn!(
                "Identité de progression non envoyée pour {}: {err}",
                player.name
            );
        }
        match client.fetch_profile(player.uuid).await {
            Ok(profile) => {
                self.store_profile(player.uuid, profile, true);
                self.refresh_single_player_ui(player).await;
            }
            Err(err) => {
                tracing::warn!(
                    "Profil de progression indisponible pour {}: {err}",
                    player.name
                );
            }
        }
    }

    fn persist_equipped(&self, player: &Player, slot: CosmeticSlot, cosmetic: Option<CosmeticId>) {
        let Some(client) = self.state.orchestrator.clone() else {
            return;
        };
        let lobby = self.clone();
        let player = player.clone();
        tokio::spawn(async move {
            match client.equip(player.uuid, slot, cosmetic).await {
                Ok(profile) => {
                    lobby.store_profile(player.uuid, profile, false);
                    lobby.refresh_single_player_ui(&player).await;
                }
                Err(err) => {
                    tracing::debug!("Équipement non persisté pour {}: {err}", player.name);
                }
            }
        });
    }

    fn open_progress_for(&self, player: Player, kind: ProgressMenuKind) {
        let lobby = self.clone();
        tokio::spawn(async move {
            if let Some(client) = &lobby.state.orchestrator
                && let Ok(profile) = client.fetch_profile(player.uuid).await
            {
                lobby.store_profile(player.uuid, profile, false);
                lobby.refresh_single_player_ui(&player).await;
            }
            let profile = lobby.state.profiles.snapshot(player.uuid);
            let claims = lobby.state.auth.claims_of(player.uuid);
            if let Err(err) = lobby
                .state
                .menus
                .open_progress(
                    &player.server(),
                    &player,
                    kind,
                    profile.as_ref(),
                    claims.as_ref(),
                )
                .await
            {
                tracing::debug!("Menu de progression non ouvert pour {}: {err}", player.name);
            }
        });
    }

    fn buy_cosmetic(&self, player: Player, cosmetic: CosmeticId) {
        let Some(client) = self.state.orchestrator.clone() else {
            player.message.send("§cBoutique indisponible.");
            return;
        };
        let lobby = self.clone();
        tokio::spawn(async move {
            match client.purchase(player.uuid, cosmetic).await {
                Ok(profile) => {
                    lobby.store_profile(player.uuid, profile, false);
                    player.sound.play(
                        SoundCue::new(Sound::EntityPlayerLevelup)
                            .volume(0.8)
                            .pitch(1.3),
                    );
                    player
                        .message
                        .send(format!("§a✔ Achat réussi : {}", cosmetic_title(cosmetic)));
                    lobby.refresh_single_player_ui(&player).await;
                }
                Err(err) => player.message.send(error_message(&err)),
            }
        });
    }

    fn equip_from_shop(&self, player: Player, slot: CosmeticSlot, cosmetic: CosmeticId) {
        let Some(client) = self.state.orchestrator.clone() else {
            player.message.send("§cBoutique indisponible.");
            return;
        };
        let lobby = self.clone();
        tokio::spawn(async move {
            match client.equip(player.uuid, slot, Some(cosmetic)).await {
                Ok(profile) => {
                    lobby.store_profile(player.uuid, profile, true);
                    player.sound.play(
                        SoundCue::new(Sound::EntityItemPickup)
                            .volume(0.75)
                            .pitch(1.2),
                    );
                    player
                        .action_bar
                        .send(format!("{} §aéquipé !", cosmetic_title(cosmetic)));
                    lobby.refresh_single_player_ui(&player).await;
                }
                Err(err) => player.message.send(error_message(&err)),
            }
        });
    }

    fn shop_item_click(&self, player: Player, cosmetic: CosmeticId) {
        let Some(entry) = Catalog::entry(cosmetic) else {
            return;
        };
        let state = self
            .state
            .profiles
            .with(player.uuid, |p| p.map(|p| shop_item_state(p, &entry)));
        match state {
            None => player.message.send("§cBoutique indisponible."),
            Some(ShopItemState::Equipped) => player
                .action_bar
                .send(format!("{} §7est déjà équipé.", cosmetic_title(cosmetic))),
            Some(ShopItemState::Unlocked) => self.equip_from_shop(player, entry.slot, cosmetic),
            Some(ShopItemState::Buyable(_)) => self.buy_cosmetic(player, cosmetic),
            Some(ShopItemState::Locked(unlock)) => {
                player.message.send(requirement_message(unlock));
            }
        }
    }

    fn handle_progress_click(&self, player: Player, kind: ProgressMenuKind, slot: i32) {
        match kind {
            ProgressMenuKind::Profile => {
                let Some(action) = decode_profile_click(slot) else {
                    return;
                };
                player
                    .sound
                    .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                match action {
                    ProfileMenuAction::OpenSettings => self.open_settings_for(player),
                    ProfileMenuAction::FerrisKey => self.ferriskey_action(player),
                    ProfileMenuAction::OpenShop => self.open_progress_for(
                        player,
                        ProgressMenuKind::Shop(CosmeticSlot::LobbyGadget),
                    ),
                    ProfileMenuAction::Close => player.inventory.close(),
                }
            }
            ProgressMenuKind::Shop(page) => match decode_shop_click(slot, page) {
                Some(ShopMenuAction::Page(tab)) => {
                    player
                        .sound
                        .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                    let claims = self.state.auth.claims_of(player.uuid);
                    self.state.profiles.with(player.uuid, |profile| {
                        self.state.menus.show_progress_page(
                            player.uuid,
                            ProgressMenuKind::Shop(tab),
                            profile,
                            claims.as_ref(),
                        );
                    });
                }
                Some(ShopMenuAction::Item(cosmetic)) => {
                    player
                        .sound
                        .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                    self.shop_item_click(player, cosmetic);
                }
                Some(ShopMenuAction::BackToCollectibles) => {
                    player
                        .sound
                        .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                    self.open_gadgets_menu_for(player);
                }
                None => {}
            },
        }
    }

    fn open_settings_for(&self, player: Player) {
        let lobby = self.clone();
        tokio::spawn(async move {
            let prefs = lobby.state.prefs.get(player.uuid);
            let _ = lobby
                .state
                .menus
                .open_options(&player.server(), &player, &prefs)
                .await;
        });
    }

    fn ferriskey_action(&self, player: Player) {
        player.inventory.close();
        if self.state.auth.is_authenticated(player.uuid) {
            let lobby = self.clone();
            tokio::spawn(async move {
                lobby.state.auth.logout(player.uuid).await;
                player
                    .message
                    .send("§eVotre compte FerrisKey a été délié de cette session.");
                lobby.refresh_single_player_ui(&player).await;
                lobby.sync_progression(&player).await;
            });
        } else {
            self.start_ferriskey_login(player);
        }
    }

    fn start_ferriskey_login(&self, player: Player) {
        let lobby = self.clone();
        tokio::spawn(async move {
            match lobby.state.auth.start_device_flow(player.uuid).await {
                Ok((device, cancel_rx)) => {
                    player.sound.play(
                        SoundCue::new(Sound::BlockNoteBlockBell)
                            .volume(0.8)
                            .pitch(1.2),
                    );
                    player.message.send_rich(build_device_prompt(&device));

                    match lobby
                        .state
                        .auth
                        .poll_device_flow(player.uuid, &device, cancel_rx)
                        .await
                    {
                        Ok(session) => {
                            player.sound.play(
                                SoundCue::new(Sound::EntityPlayerLevelup)
                                    .volume(0.9)
                                    .pitch(1.15),
                            );
                            player.title.send(
                                Title::new("§a§l✔ FERRISKEY LIÉ").subtitle(format!(
                                    "§fBienvenue, §b{}",
                                    session.claims.username
                                )),
                            );
                            player.message.send(format!(
                                "§a✔ Votre compte FerrisKey (§f{}§a) est désormais actif !",
                                session.claims.username
                            ));
                            lobby.refresh_single_player_ui(&player).await;
                            lobby.sync_progression(&player).await;
                        }
                        Err(AuthError::Cancelled) => {}
                        Err(AuthError::Expired) => {
                            player.message.send(
                                "§e⌛ Le lien FerrisKey a expiré. Tapez §f/login §epour recommencer.",
                            );
                        }
                        Err(AuthError::Denied) => {
                            player
                                .message
                                .send("§c✖ Connexion FerrisKey annulée dans le navigateur.");
                        }
                        Err(err) => {
                            tracing::warn!(
                                "Échec du polling Device Flow pour {}: {err}",
                                player.name
                            );
                            player.message.send(format!("§c{err}"));
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!("Impossible d'initier FerrisKey pour {}: {err}", player.name);
                    let detail = err.to_string();
                    player.message.send(format!(
                        "§cImpossible de joindre FerrisKey ({}) : {detail}",
                        lobby.state.config.ferriskey_base_url
                    ));
                    if detail.contains("unauthorized_client") {
                        player.message.send(format!(
                            "§eAstuce : vérifiez dans FerrisKey (realm §f{}§e → client §f{}§e) que §aDevice Code Grant §eest activé.",
                            lobby.state.config.ferriskey_realm,
                            lobby.state.config.ferriskey_client_id
                        ));
                    }
                }
            }
        });
    }

    fn start_capitainerie_login(&self, player: Player) {
        if !self.state.auth.uses_separate_capitainerie_auth() {
            self.start_ferriskey_login(player);
            return;
        }
        let lobby = self.clone();
        tokio::spawn(async move {
            let realm = lobby.state.config.capitainerie_realm.clone();
            match lobby
                .state
                .auth
                .start_capitainerie_device_flow(player.uuid)
                .await
            {
                Ok((device, cancel_rx)) => {
                    player.sound.play(
                        SoundCue::new(Sound::BlockNoteBlockBell)
                            .volume(0.8)
                            .pitch(1.2),
                    );
                    let prompt_label = format!("Capitainerie (realm {realm})");
                    player
                        .message
                        .send_rich(build_device_prompt_for(&device, Some(&prompt_label)));

                    match lobby
                        .state
                        .auth
                        .poll_capitainerie_device_flow(player.uuid, &device, cancel_rx)
                        .await
                    {
                        Ok(session) => {
                            player.sound.play(
                                SoundCue::new(Sound::EntityPlayerLevelup)
                                    .volume(0.9)
                                    .pitch(1.15),
                            );
                            player.title.send(
                                Title::new("§a§l✔ CAPITAINERIE LIÉE")
                                    .subtitle(format!("§fCompte : §b{}", session.claims.username)),
                            );
                            player.message.send(format!(
                                "§a✔ Votre compte Capitainerie (§f{}§a, realm §b{realm}§a) est désormais actif !",
                                session.claims.username
                            ));
                        }
                        Err(AuthError::Cancelled) => {}
                        Err(AuthError::Expired) => {
                            player.message.send(
                                "§e⌛ Le lien Capitainerie a expiré. Tapez §f/caplogin §epour recommencer.",
                            );
                        }
                        Err(AuthError::Denied) => {
                            player
                                .message
                                .send("§c✖ Connexion Capitainerie annulée dans le navigateur.");
                        }
                        Err(err) => {
                            tracing::warn!(
                                "Échec du polling Device Flow Capitainerie pour {}: {err}",
                                player.name
                            );
                            player.message.send(format!("§c{err}"));
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        "Impossible d'initier FerrisKey Capitainerie pour {}: {err}",
                        player.name
                    );
                    let detail = err.to_string();
                    player.message.send(format!(
                        "§cImpossible de joindre FerrisKey ({}) pour Capitainerie : {detail}",
                        lobby.state.config.capitainerie_ferriskey_url
                    ));
                    if detail.contains("unauthorized_client") || detail.contains("invalid_client") {
                        player.message.send(format!(
                            "§eAstuce : vérifiez dans FerrisKey (realm §f{realm}§e → client §f{}§e) que §aDevice Code Grant §eest activé.",
                            lobby.state.config.capitainerie_client_id
                        ));
                    }
                }
            }
        });
    }

    fn prompt_login_required(&self, player: &Player, context_label: &str) {
        player.sound.play(
            SoundCue::new(Sound::BlockNoteBlockBell)
                .volume(0.8)
                .pitch(0.8),
        );
        player.dialog.show(
            Dialog::new("§6§lAuthentification FerrisKey requise")
                .line(format!(
                    "§7Pour utiliser §f{context_label}§7, connectez votre compte personnel FerrisKey."
                ))
                .line("§7Aucun mot de passe n'est saisi dans Minecraft : vous validez dans le navigateur.")
                .button(DialogButton::new(
                    "§a🔐 Se connecter avec FerrisKey",
                    LOGIN_BUTTON_ID,
                ))
                .button(DialogButton::new("§7Plus tard", CANCEL_BUTTON_ID)),
        );
    }

    fn prompt_capitainerie_login_required(&self, player: &Player, context_label: &str) {
        if !self.state.auth.uses_separate_capitainerie_auth() {
            self.prompt_login_required(player, context_label);
            return;
        }
        player.sound.play(
            SoundCue::new(Sound::BlockNoteBlockBell)
                .volume(0.8)
                .pitch(0.8),
        );
        player.dialog.show(
            Dialog::new("§6§lConnexion Capitainerie requise")
                .line(format!(
                    "§7Pour utiliser §f{context_label}§7, connectez votre compte du realm §b{}§7.",
                    self.state.config.capitainerie_realm
                ))
                .line("§7Cette session Capitainerie est indépendante de votre compte Lobby.")
                .button(DialogButton::new(
                    "§a🔐 Se connecter à Capitainerie",
                    CAP_LOGIN_BUTTON_ID,
                ))
                .button(DialogButton::new("§7Plus tard", CANCEL_BUTTON_ID)),
        );
    }

    fn portal_blast(&self, player: Player, at: Position, entry_step: Option<Vector>, yaw: f32) {
        let launch = portal_launch_velocity(entry_step, yaw);
        tokio::spawn(async move {
            let world = &player.world;
            world
                .explosion
                .create(Explosion::new(at, PORTAL_BLAST_POWER, false));
            world.particle.spawn(
                ParticleBurst::new(Particle::ExplosionEmitter, at)
                    .count(6)
                    .spread(Vector {
                        x: 1.8,
                        y: 1.2,
                        z: 1.8,
                    }),
            );
            world.particle.spawn(
                ParticleBurst::new(Particle::Portal, at)
                    .count(160)
                    .spread(Vector {
                        x: 1.5,
                        y: 1.5,
                        z: 1.5,
                    })
                    .speed(0.9),
            );
            player.sound.play(
                SoundCue::new(Sound::EntityGenericExplode)
                    .volume(1.4)
                    .pitch(0.8),
            );
            tokio::time::sleep(PORTAL_LAUNCH_DELAY).await;
            player.movement.set_velocity(launch);
        });
    }

    fn join_sheepwars(&self, player: Player) {
        self.join_game(player, GameKind::SheepWars, 0);
    }

    fn join_deacoudre(&self, player: Player) {
        self.join_game(player, GameKind::DeACoudre, 0);
    }

    fn join_towerwars(&self, player: Player) {
        let lobby = self.clone();
        tokio::spawn(async move {
            let queued = match &lobby.state.orchestrator {
                Some(orch) => orch
                    .is_queued(player.uuid, GameKind::TowerWars)
                    .await
                    .unwrap_or(true),
                None => true,
            };
            if queued {
                lobby.join_game(player, GameKind::TowerWars, 0);
                return;
            }
            let _ = lobby
                .state
                .menus
                .open_towerwars_size(&player.server(), &player)
                .await;
        });
    }

    fn join_game(&self, player: Player, game: GameKind, match_size: u8) {
        let lobby = self.clone();
        tokio::spawn(async move {
            player.inventory.close();
            let tag = game_tag(game);
            let name = game_name(game);
            if let Some(orch) = lobby.state.orchestrator.clone() {
                match orch
                    .toggle_queue(player.uuid, &player.name, game, match_size)
                    .await
                {
                    Ok(ToggleQueueOutcome::Joined(resp))
                        if resp.status == QueueJoinStatus::NotPartyLeader =>
                    {
                        player
                            .message
                            .send(not_party_leader(tag, resp.target_server.as_deref()));
                    }
                    Ok(ToggleQueueOutcome::Joined(resp)) => {
                        let counts = match orch.fetch_status().await {
                            Ok(snap) => QueueCounts::from_snapshot(&snap),
                            Err(_) => orch.cached_queue_counts().with(game, resp.total_queued),
                        };
                        lobby.state.menus.refresh_games_menu(counts);
                        lobby.state.npcs.update_pedestal_labels(counts);
                        if resp.status == QueueJoinStatus::Transferring {
                            player.sound.play(
                                SoundCue::new(Sound::EntityEndermanTeleport)
                                    .volume(0.85)
                                    .pitch(1.15),
                            );
                            let target = resp.target_server.as_deref().unwrap_or(game.slug());
                            player.message.send(format!(
                                "{tag} §8» §aPartie trouvée ! Transfert vers §f{target}§a…"
                            ));
                        } else {
                            player.sound.play(
                                SoundCue::new(Sound::EntityExperienceOrbPickup)
                                    .volume(0.8)
                                    .pitch(1.3),
                            );
                            let size = queue_size_hint(game, resp.min_players, resp.total_queued);
                            player.action_bar.send(format!(
                                "{tag} §8• §aFile #{}/{}{}{size} §7(§b{}/{} srv§7)",
                                resp.position,
                                resp.total_queued.max(1),
                                queue_min_hint(game, resp.min_players),
                                resp.ready_servers,
                                resp.max_servers
                            ));
                            player.message.send(format!(
                                "{tag} §8» §aFile d'attente rejointe{size} (§f#{}§a/§f{}§a{} — §b{}§a prêt(s) / §f{} max§a). Recliquez pour quitter.",
                                resp.position,
                                resp.total_queued,
                                queue_min_hint(game, resp.min_players),
                                resp.ready_servers,
                                resp.max_servers
                            ));
                        }
                        lobby.refresh_single_player_ui(&player).await;
                    }
                    Ok(ToggleQueueOutcome::Left(resp)) => {
                        let counts = match orch.fetch_status().await {
                            Ok(snap) => QueueCounts::from_snapshot(&snap),
                            Err(_) => orch.cached_queue_counts().with(game, resp.total_queued),
                        };
                        lobby.state.menus.refresh_games_menu(counts);
                        lobby.state.npcs.update_pedestal_labels(counts);
                        player
                            .message
                            .send(format!("{tag} §8» §7Vous avez quitté la file d'attente."));
                        lobby.refresh_single_player_ui(&player).await;
                    }
                    Err(err) => {
                        tracing::warn!("Orchestrateur {name} injoignable : {err}");
                        player.message.send(format!(
                            "§cService de file d'attente {name} indisponible ({err})."
                        ));
                    }
                }
                return;
            }

            if game != GameKind::SheepWars {
                player.message.send(format!(
                    "§cService de file d'attente {name} indisponible (orchestrateur non configuré)."
                ));
                return;
            }

            if let Some((host, port)) = lobby.state.config.sheepwars_transfer.clone() {
                let probe_target = lobby
                    .state
                    .config
                    .sheepwars_backend
                    .clone()
                    .unwrap_or_else(|| format!("{host}:{port}"));
                let backend_up = tokio::time::timeout(
                    Duration::from_millis(250),
                    tokio::net::TcpStream::connect(&probe_target),
                )
                .await
                .is_ok_and(|res| res.is_ok());

                if backend_up {
                    player
                        .message
                        .send("§d§lSheepWars §8» §7Transfert vers le serveur §fSheepWars§7…");
                    if let Err(err) = player.connection.transfer(host, port).await {
                        player.message.send(format!(
                            "§cImpossible de transférer vers SheepWars ({err})."
                        ));
                    }
                    return;
                }
            }

            if let Some(world_name) = lobby.state.config.sheepwars_world.clone() {
                let target_loc = Location {
                    world: world_name.clone(),
                    position: Position {
                        x: 0.5,
                        y: 65.0,
                        z: 0.5,
                    },
                    rotation: Rotation::default(),
                };
                if player.location.teleport_to(&target_loc).await.is_ok() {
                    player
                        .sound
                        .play(SoundCue::new(Sound::EntityEndermanTeleport));
                    player
                        .message
                        .send("§d§lSheepWars §8» §aBienvenue dans l'arène !");
                    return;
                }
            }

            // Local waiting queue fallback while colleague builds the SheepWars arena plugin
            let (joined, count) = {
                let mut q = lobby
                    .state
                    .sheepwars_queue
                    .write()
                    .unwrap_or_else(|e| e.into_inner());
                let joined = if q.contains(&player.uuid) {
                    q.remove(&player.uuid);
                    false
                } else {
                    q.insert(player.uuid);
                    true
                };
                (joined, q.len())
            };

            let counts = QueueCounts::default().with(GameKind::SheepWars, count);
            lobby.state.menus.refresh_games_menu(counts);
            lobby.state.npcs.update_pedestal_labels(counts);

            if joined {
                player.sound.play(
                    SoundCue::new(Sound::EntityExperienceOrbPickup)
                        .volume(0.8)
                        .pitch(1.3),
                );
                player.action_bar.send(format!(
                    "§d§lSheepWars §8• §aFile d'attente rejointe (§f{count} §ajoueur(s))"
                ));
                player.message.send(format!(
                    "§d§lSheepWars §8» §aVous êtes inscrit dans la file d'attente (§f{count}§a prêt(s)). Recliquez pour quitter."
                ));
            } else {
                player
                    .message
                    .send("§d§lSheepWars §8» §7Vous avez quitté la file d'attente.");
            }
            lobby.refresh_single_player_ui(&player).await;
        });
    }

    fn teleport_to_poi(&self, player: Player, poi_key: &str) {
        let layout = self.state.layout.snapshot();
        let target = layout.pois.get(poi_key).copied().unwrap_or(layout.spawn);
        tokio::spawn(async move {
            player.inventory.close();
            if player
                .location
                .teleport_facing(target.facing())
                .await
                .is_ok()
            {
                player.sound.play(
                    SoundCue::new(Sound::EntityEndermanTeleport)
                        .volume(0.65)
                        .pitch(1.1),
                );
            }
        });
    }

    fn toggle_visibility(&self, player: Player) {
        let lobby = self.clone();
        tokio::spawn(async move {
            let prefs = lobby
                .state
                .prefs
                .update(player.uuid, |p| p.players_visible = !p.players_visible);

            if let Ok(others) = player.server().get_players().await {
                for other in others {
                    if other.uuid == player.uuid {
                        continue;
                    }
                    if prefs.players_visible {
                        player.visibility.show(&other);
                    } else {
                        player.visibility.hide(&other);
                    }
                }
            }

            apply_lobby_hotbar(&player, &prefs);
            lobby.state.menus.refresh_open_options(player.uuid, &prefs);

            player.sound.play(
                SoundCue::new(Sound::UiButtonClick)
                    .volume(0.7)
                    .pitch(if prefs.players_visible { 1.3 } else { 0.8 }),
            );
            player.action_bar.send(if prefs.players_visible {
                "§a👁 Joueurs du Hub affichés"
            } else {
                "§7👁 Joueurs du Hub masqués"
            });
        });
    }

    fn toggle_flight_for(&self, player: &Player) {
        let updated = self
            .state
            .prefs
            .update(player.uuid, |p| p.flight_enabled = !p.flight_enabled);
        updated.apply_movement(player);
        self.state.menus.refresh_open_options(player.uuid, &updated);

        if updated.flight_enabled {
            player.sound.play(
                SoundCue::new(Sound::EntityBatTakeoff)
                    .volume(0.75)
                    .pitch(1.2),
            );
            player.world.particle.spawn(
                ParticleBurst::new(Particle::Cloud, player.position)
                    .count(12)
                    .spread(Vector {
                        x: 0.35,
                        y: 0.25,
                        z: 0.35,
                    })
                    .speed(0.05),
            );
            player
                .action_bar
                .send("§b✈ Vol dans le Lobby ACTIVÉ §7(Double-espace)");
            player.message.send(
                "§b✈ §aVol dans le Lobby activé ! Appuyez deux fois sur Espace pour décoller.",
            );
        } else {
            player
                .sound
                .play(SoundCue::new(Sound::UiButtonClick).volume(0.7).pitch(0.75));
            player.action_bar.send("§7✈ Vol dans le Lobby DÉSACTIVÉ");
            player.message.send("§7✈ Vol dans le Lobby désactivé.");
        }
    }

    fn cycle_speed_for(&self, player: &Player) {
        let updated = self
            .state
            .prefs
            .update(player.uuid, |p| p.speed_tier = p.speed_tier.next());
        updated.apply_movement(player);
        self.state.menus.refresh_open_options(player.uuid, &updated);

        player.sound.play(
            SoundCue::new(Sound::EntityExperienceOrbPickup)
                .volume(0.75)
                .pitch(1.25),
        );
        player
            .action_bar
            .send(format!("§e⚡ Vitesse : {}", updated.speed_tier.label()));
        player.message.send(format!(
            "§e⚡ Vitesse réglée sur : {}",
            updated.speed_tier.label()
        ));
    }

    fn cycle_trail_for(&self, player: &Player) {
        let current = self.state.prefs.get(player.uuid).aura_trail;
        let next = self
            .state
            .profiles
            .with(player.uuid, |profile| next_usable_trail(current, profile));
        if next == current {
            player
                .message
                .send("§7Aucune autre traînée débloquée. Voir §e/boutique§7.");
            return;
        }
        let updated = self
            .state
            .prefs
            .update(player.uuid, |p| p.aura_trail = next);
        self.state.profiles.with(player.uuid, |profile| {
            self.state
                .menus
                .refresh_open_gadgets(player.uuid, &updated, profile);
        });
        self.persist_equipped(player, CosmeticSlot::LobbyTrail, next.cosmetic());
        player.sound.play(
            SoundCue::new(Sound::EntityExperienceOrbPickup)
                .volume(0.75)
                .pitch(1.35),
        );
        if let Some(particle) = updated.aura_trail.particle() {
            player.world.particle.spawn(
                ParticleBurst::new(particle, player.position)
                    .count(10)
                    .spread(Vector {
                        x: 0.35,
                        y: 0.4,
                        z: 0.35,
                    })
                    .speed(0.03),
            );
        }
        player
            .action_bar
            .send(format!("§d✦ Traînée : {}", updated.aura_trail.label()));
    }

    fn equip_gadget_for(&self, player: &Player, gadget: Option<LobbyGadget>) {
        if let Some(gadget) = gadget
            && !self
                .state
                .profiles
                .with(player.uuid, |p| gadget_usable(p, gadget))
        {
            player
                .message
                .send(locked_cosmetic_message(gadget.cosmetic()));
            return;
        }
        let updated = self
            .state
            .prefs
            .update(player.uuid, |p| p.equipped_gadget = gadget);
        apply_lobby_hotbar(player, &updated);
        self.state.profiles.with(player.uuid, |profile| {
            self.state
                .menus
                .refresh_open_gadgets(player.uuid, &updated, profile);
        });
        self.persist_equipped(
            player,
            CosmeticSlot::LobbyGadget,
            gadget.map(LobbyGadget::cosmetic),
        );

        player.sound.play(
            SoundCue::new(Sound::EntityItemPickup)
                .volume(0.75)
                .pitch(1.2),
        );
        match gadget {
            Some(g) => {
                player
                    .action_bar
                    .send(format!("{} §aéquipé ! §7(Slot 6 · Clic droit)", g.title()));
                player.message.send(format!(
                    "§b✦ Gadget équipé : {} §7— clic droit en main (slot 6) pour l'utiliser !",
                    g.title()
                ));
            }
            None => {
                player.action_bar.send("§7Gadget retiré de votre barre.");
            }
        }
    }

    fn open_gadgets_menu_for(&self, player: Player) {
        let lobby = self.clone();
        tokio::spawn(async move {
            let cur_prefs = lobby.state.prefs.get(player.uuid);
            let profile = lobby.state.profiles.snapshot(player.uuid);
            let _ = lobby
                .state
                .menus
                .open_gadgets(&player.server(), &player, &cur_prefs, profile.as_ref())
                .await;
        });
    }

    fn handle_menu_click(&self, player: Player, clicked_inv: Option<&InventoryRef>, slot: i32) {
        if self.state.menus.is_games_inventory(clicked_inv) {
            if let Some(action) = decode_games_click(slot) {
                player
                    .sound
                    .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                match action {
                    GamesMenuAction::JoinSheepWars => self.join_sheepwars(player),
                    GamesMenuAction::JoinTowerWars => self.join_towerwars(player),
                    GamesMenuAction::JoinDeACoudre => self.join_deacoudre(player),
                    GamesMenuAction::TeleportPoi(poi) => self.teleport_to_poi(player, poi),
                    GamesMenuAction::Close => player.inventory.close(),
                }
            }
            return;
        }

        if self
            .state
            .menus
            .is_player_options_inventory(player.uuid, clicked_inv)
        {
            if let Some(action) = decode_options_click(slot) {
                player
                    .sound
                    .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                match action {
                    OptionsMenuAction::ToggleVisibility => self.toggle_visibility(player),
                    OptionsMenuAction::ToggleFlight => self.toggle_flight_for(&player),
                    OptionsMenuAction::CycleSpeed => self.cycle_speed_for(&player),
                    OptionsMenuAction::ToggleChatSounds => {
                        let updated = self
                            .state
                            .prefs
                            .update(player.uuid, |p| p.chat_sounds = !p.chat_sounds);
                        self.state.menus.refresh_open_options(player.uuid, &updated);
                        player.action_bar.send(if updated.chat_sounds {
                            "§a🔔 Sons & Mentions tchat activés"
                        } else {
                            "§7🔕 Sons & Mentions tchat coupés"
                        });
                    }
                    OptionsMenuAction::BackToProfile => {
                        self.open_progress_for(player, ProgressMenuKind::Profile);
                    }
                }
            }
            return;
        }

        if self
            .state
            .menus
            .is_player_towerwars_size_inventory(player.uuid, clicked_inv)
        {
            if let Some(size) = decode_towerwars_size_click(slot) {
                player
                    .sound
                    .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                self.join_game(player, GameKind::TowerWars, size);
            }
            return;
        }

        if let Some(kind) = self
            .state
            .menus
            .progress_menu_kind(player.uuid, clicked_inv)
        {
            self.handle_progress_click(player, kind, slot);
            return;
        }

        if let Some(ids) = self
            .state
            .menus
            .servers_menu_server_ids(player.uuid, clicked_inv)
        {
            if let Some(index) = decode_servers_slot(slot, ids.len())
                && let Some(server_id) = ids[index].clone()
            {
                player
                    .sound
                    .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                player.inventory.close();
                let lobby = self.clone();
                tokio::spawn(async move {
                    lobby.spectate_server(&player, &server_id).await;
                });
            }
            return;
        }

        if self
            .state
            .menus
            .is_player_gadgets_inventory(player.uuid, clicked_inv)
            && let Some(action) = decode_gadgets_click(slot)
        {
            player
                .sound
                .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
            match action {
                GadgetsMenuAction::Equip(gadget) => {
                    self.equip_gadget_for(&player, Some(gadget));
                }
                GadgetsMenuAction::UnequipGadget => {
                    self.equip_gadget_for(&player, None);
                }
                GadgetsMenuAction::CycleTrail => self.cycle_trail_for(&player),
                GadgetsMenuAction::OpenShop => self
                    .open_progress_for(player, ProgressMenuKind::Shop(CosmeticSlot::LobbyGadget)),
                GadgetsMenuAction::Close => player.inventory.close(),
            }
        }
    }

    async fn spectate_server(&self, player: &Player, server_id: &str) {
        let Some(client) = &self.state.orchestrator else {
            player.message.send("§cOrchestrateur non configuré.");
            return;
        };
        match client.spectate(server_id, player.uuid).await {
            Ok(()) => player
                .message
                .send(format!("§aTransfert vers {server_id}…")),
            Err(OrchestratorClientError::ServerNotFound) => player
                .message
                .send(format!("§cServeur {server_id} introuvable.")),
            Err(OrchestratorClientError::ServerNotJoinable) => player.message.send(format!(
                "§c{server_id} n'est pas observable pour le moment."
            )),
            Err(err) => player
                .message
                .send(format!("§cErreur orchestrateur : {err}")),
        }
    }

    async fn is_staff(&self, player: &Player) -> bool {
        let claims = self.state.auth.claims_of(player.uuid);
        let is_op = player.get_state().await.map(|st| st.op).unwrap_or(false);
        let has_admin = player.permissions.has("hypolia.admin");
        matches!(
            Rank::resolve(claims.as_ref(), is_op, has_admin),
            Rank::Admin | Rank::Dev
        )
    }

    fn show_whoami(&self, player: &Player) {
        let mut msg =
            RichMessage::new().text("§8§m                                                  \n");
        match self.state.auth.claims_of(player.uuid) {
            Some(claims) => {
                let has_admin = player.permissions.has("hypolia.admin");
                let rank = Rank::resolve(Some(&claims), false, has_admin);
                let roles = if claims.roles.is_empty() {
                    "aucun rôle spécifique".to_owned()
                } else {
                    claims.roles.join(", ")
                };
                msg = msg
                    .text(format!("§a§l✔ Profil FerrisKey — §f{}\n", claims.username))
                    .text(format!("§7Grade Lobby : {}\n", rank.sidebar_label()))
                    .text(format!(
                        "§7E-mail : §f{}\n",
                        claims.email.as_deref().unwrap_or("non exposé")
                    ))
                    .text(format!("§7Rôles OIDC : §b{roles}\n  "))
                    .part(
                        Part::new("§c[Déconnecter ce compte]")
                            .hover("§7Cliquez pour exécuter /logout")
                            .click(Click::RunCommand("/logout".to_owned())),
                    );
            }
            None => {
                msg = msg.text("§eAucun compte FerrisKey Lobby n'est lié. ").part(
                    Part::new("§a§l[Se connecter]")
                        .hover("§7Lancer la connexion FerrisKey (/login)")
                        .click(Click::RunCommand("/login".to_owned())),
                );
            }
        }

        if self.state.npcs.has_capitainerie() && self.state.auth.uses_separate_capitainerie_auth() {
            let realm = &self.state.config.capitainerie_realm;
            msg = msg.text(format!("\n§7Capitainerie (realm §b{realm}§7) : "));
            match self.state.auth.capitainerie_claims_of(player.uuid) {
                Some(cap_claims) => {
                    msg = msg.text(format!("§a✔ {} ", cap_claims.username)).part(
                        Part::new("§c[Délier]")
                            .hover("§7Cliquez pour exécuter /caplogout")
                            .click(Click::RunCommand("/caplogout".to_owned())),
                    );
                }
                None => {
                    msg = msg.text("§eNon lié ").part(
                        Part::new("§b[/caplogin]")
                            .hover("§7Connecter votre compte Capitainerie")
                            .click(Click::RunCommand("/caplogin".to_owned())),
                    );
                }
            }
        }

        msg = msg.text("\n§8§m                                                  ");
        player.message.send_rich(msg);
    }

    async fn suggest_lobby_subcommands(&self, _player: &Player, typed: &str) -> Vec<String> {
        LOBBY_SUBCOMMANDS
            .iter()
            .filter(|cmd| cmd.starts_with(typed))
            .map(|cmd| (*cmd).to_owned())
            .collect()
    }

    fn store_task_result(&self, key: String, report: TaskResultReport) {
        let mut guard = self
            .state
            .task_results
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if guard.len() >= MAX_CACHED_RESULTS
            && !guard.contains_key(&key)
            && let Some(evict) = guard.keys().next().cloned()
        {
            guard.remove(&evict);
        }
        guard.insert(key, report);
    }

    fn spawn_run_watcher(
        &self,
        player: Player,
        agent: TeamAgent,
        task_title: String,
        started: RunStarted,
    ) {
        let lobby = self.clone();
        tokio::spawn(async move {
            let Some(client) = lobby.state.npcs.capitainerie_client() else {
                return;
            };
            let server = player.server();
            let short_title = clip_text(&task_title, 28);
            let bar = server
                .boss_bars
                .create(
                    format!("§e⏳ {} §8• §f{short_title}", agent.name),
                    BarColor::Yellow,
                    BarStyle::Segmented10,
                    &[],
                )
                .await
                .ok();
            if let Some(bar) = &bar {
                bar.add_player(&player);
                bar.set_progress(0.08);
            }
            let mut attached_entity = Some(player.entity_id);

            let mut consecutive_errors = 0usize;
            for _ in 0..RUN_WATCH_MAX_POLLS {
                tokio::time::sleep(RUN_WATCH_POLL_INTERVAL).await;

                let online_player = server
                    .get_players()
                    .await
                    .ok()
                    .and_then(|list| list.into_iter().find(|p| p.uuid == player.uuid));

                match (&online_player, &bar) {
                    (Some(p), Some(bar)) if attached_entity != Some(p.entity_id) => {
                        bar.add_player(p);
                        attached_entity = Some(p.entity_id);
                    }
                    (None, _) => attached_entity = None,
                    _ => {}
                }

                let Ok(bearer) = lobby.state.auth.valid_capitainerie_token(player.uuid).await
                else {
                    break;
                };

                let detail = match client.run(&bearer, &started.run_id).await {
                    Ok(d) => {
                        consecutive_errors = 0;
                        d
                    }
                    Err(err) => {
                        tracing::debug!(
                            "Suivi run Capitainerie {} interrompu/transitoire : {err}",
                            started.run_id
                        );
                        consecutive_errors += 1;
                        if consecutive_errors >= 5 {
                            break;
                        }
                        continue;
                    }
                };

                if !detail.is_terminal() {
                    let progress_info = lobby.state.npcs.run_progress(&started.run_id);
                    if detail.status == "queued" {
                        if let Some(bar) = &bar {
                            bar.set_color(BarColor::Yellow);
                            bar.set_progress(0.08);
                            bar.set_title(format!(
                                "§e⏳ {} §8• §7En file — §f{short_title}",
                                agent.name
                            ));
                        }
                        if let Some(p) = &online_player {
                            p.action_bar.send(format!(
                                "§e⏳ {} §8• §7En file d'attente : §f{}",
                                agent.name,
                                clip_text(&task_title, 36)
                            ));
                        }
                    } else {
                        let (step_suffix, ratio) = match progress_info {
                            Some((step_kind, done, total)) => {
                                let step_text = step_kind
                                    .as_deref()
                                    .map(|k| format!(" §7· {}", step_name(k)))
                                    .unwrap_or_default();
                                let frac = if total > 0 {
                                    (f64::from(done) / f64::from(total)).clamp(0.12, 0.95)
                                } else {
                                    0.45
                                };
                                (step_text, frac)
                            }
                            None => (String::new(), 0.45),
                        };
                        if let Some(bar) = &bar {
                            bar.set_color(BarColor::Blue);
                            bar.set_progress(ratio);
                            bar.set_title(format!(
                                "§b⚒ {}{} §8• §f{short_title}",
                                agent.name, step_suffix
                            ));
                        }
                        if let Some(p) = &online_player {
                            p.action_bar.send(format!(
                                "§b⚒ {}{} §8• §f{}",
                                agent.name,
                                step_suffix,
                                clip_text(&task_title, 36)
                            ));
                        }
                    }
                    continue;
                }

                let card_id = if !started.card_id.is_empty() {
                    started.card_id.clone()
                } else {
                    detail.card_id.clone().unwrap_or_default()
                };
                let card = if !card_id.is_empty() {
                    client.card(&bearer, &card_id).await.ok()
                } else {
                    None
                };

                let summary_text = if detail.is_succeeded() {
                    card.as_ref()
                        .and_then(|c| extract_agent_reply(c, &agent.slug))
                        .unwrap_or_else(|| "La tâche a été exécutée avec succès.".to_owned())
                } else {
                    detail
                        .failure
                        .clone()
                        .filter(|f| !f.trim().is_empty())
                        .or_else(|| {
                            card.as_ref()
                                .and_then(|c| extract_agent_reply(c, &agent.slug))
                        })
                        .unwrap_or_else(|| {
                            "La mission s'est arrêtée sans détail d'erreur.".to_owned()
                        })
                };

                let resolved_title = detail
                    .title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| task_title.clone());
                let console_url = lobby.state.config.console_task_url(&card_id);
                let report = TaskResultReport {
                    run_id: started.run_id.clone(),
                    card_id,
                    agent_name: agent.name.clone(),
                    agent_slug: agent.slug.clone(),
                    task_title: resolved_title.clone(),
                    status: detail.status.clone(),
                    summary_text,
                    console_url,
                };
                let click_key = click_slug(&started.run_id);
                lobby.store_task_result(click_key.clone(), report.clone());

                let clipped_title = clip_text(&resolved_title, 32);
                if let Some(bar) = &bar {
                    match detail.status.as_str() {
                        "succeeded" => {
                            bar.set_color(BarColor::Green);
                            bar.set_progress(1.0);
                            bar.set_title(format!(
                                "§a§l✔ {} — Terminé : §f{clipped_title}",
                                agent.name
                            ));
                        }
                        "cancelled" => {
                            bar.set_color(BarColor::Yellow);
                            bar.set_progress(1.0);
                            bar.set_title(format!(
                                "§e⚠ {} — Annulé : §f{clipped_title}",
                                agent.name
                            ));
                        }
                        _ => {
                            bar.set_color(BarColor::Red);
                            bar.set_progress(1.0);
                            bar.set_title(format!(
                                "§c§l✖ {} — Échoué : §f{clipped_title}",
                                agent.name
                            ));
                        }
                    }
                }

                if let Some(pos) = lobby.state.npcs.agent_position(&agent.id).await {
                    let burst_pos = Position {
                        x: pos.x,
                        y: pos.y + 1.4,
                        z: pos.z,
                    };
                    if detail.is_succeeded() {
                        server.world.particle.spawn(
                            ParticleBurst::new(Particle::HappyVillager, burst_pos)
                                .count(18)
                                .spread(Vector {
                                    x: 0.45,
                                    y: 0.45,
                                    z: 0.45,
                                })
                                .speed(0.05),
                        );
                        server.world.particle.spawn(
                            ParticleBurst::new(Particle::EndRod, burst_pos)
                                .count(10)
                                .spread(Vector {
                                    x: 0.35,
                                    y: 0.4,
                                    z: 0.35,
                                })
                                .speed(0.08),
                        );
                    } else {
                        server.world.particle.spawn(
                            ParticleBurst::new(Particle::Smoke, burst_pos)
                                .count(16)
                                .spread(Vector {
                                    x: 0.35,
                                    y: 0.35,
                                    z: 0.35,
                                })
                                .speed(0.04),
                        );
                    }
                }

                if let Some(p) = online_player {
                    if detail.is_succeeded() {
                        p.sound.play(
                            SoundCue::new(Sound::EntityPlayerLevelup)
                                .volume(0.9)
                                .pitch(1.15),
                        );
                        p.title.send(
                            Title::new(format!("§a§l✔ {} A TERMINÉ", agent.name))
                                .subtitle(format!("§f{clipped_title}"))
                                .times(10, 60, 20),
                        );
                        p.action_bar
                            .send(format!("§a✔ {} a terminé : §f{clipped_title}", agent.name));
                    } else if detail.status == "cancelled" {
                        p.sound.play(
                            SoundCue::new(Sound::EntityVillagerNo)
                                .volume(0.85)
                                .pitch(1.0),
                        );
                        p.title.send(
                            Title::new(format!("§e⚠ {} — ANNULÉ", agent.name))
                                .subtitle(format!("§f{clipped_title}"))
                                .times(10, 50, 20),
                        );
                        p.action_bar
                            .send(format!("§e⚠ {} — mission annulée", agent.name));
                    } else {
                        p.sound.play(
                            SoundCue::new(Sound::EntityVillagerNo)
                                .volume(0.9)
                                .pitch(0.9),
                        );
                        p.title.send(
                            Title::new(format!("§c§l✖ {} — ÉCHEC", agent.name))
                                .subtitle(format!("§f{clipped_title}"))
                                .times(10, 60, 20),
                        );
                        p.action_bar
                            .send(format!("§c✖ {} — mission échouée", agent.name));
                    }
                    p.message
                        .send_rich(build_run_completion_message(&report, &click_key));
                }

                tokio::time::sleep(RUN_FINISH_BAR_LINGER).await;
                break;
            }

            if let Some(bar) = bar {
                bar.remove();
            }
        });
    }
}

const fn game_tag(game: GameKind) -> &'static str {
    match game {
        GameKind::SheepWars => "§d§lSheepWars",
        GameKind::TowerWars => "§b§lTowerWars",
        GameKind::DeACoudre => "§6§lDé à coudre",
    }
}

const fn game_name(game: GameKind) -> &'static str {
    game_label(game)
}

fn queue_size_hint(game: GameKind, min_players: usize, total_queued: usize) -> String {
    if game != GameKind::TowerWars {
        return String::new();
    }
    let name = match min_players {
        2 => "Solo",
        4 => "Duo",
        _ => return String::new(),
    };
    format!(
        " §b({name} — {}/{min_players})§a",
        total_queued.min(min_players)
    )
}

fn queue_min_hint(game: GameKind, min_players: usize) -> String {
    if game != GameKind::TowerWars && min_players > 1 {
        format!(" §7(min §f{min_players}§7)")
    } else {
        String::new()
    }
}

fn json_field(payload: &serde_json::Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

#[listeners]
impl HypoliaLobby {
    async fn on_enable(&self, server: &Server) {
        server.world.pvp.set(false);
        server.world.difficulty.set(Difficulty::Peaceful);
        server.world.weather.set(Weather::Clear);
        server.world.time.set(6000);

        // Lock environmental rules so custom trees, braziers, ice, and decorations never decay or burn
        server.command.dispatch("gamerule random_tick_speed 0");
        server
            .command
            .dispatch("gamerule fire_spread_radius_around_player 0");
        server.command.dispatch("gamerule advance_time false");
        server.command.dispatch("gamerule mob_griefing false");

        if let Err(err) = self.state.menus.init(server).await {
            tracing::warn!("Impossible de créer le menu des jeux : {err}");
        }

        self.state
            .npcs
            .spawn_all(server, QueueCounts::default())
            .await;

        let leaderboard = Arc::clone(&self.state.leaderboard);
        let srv_leaderboard = server.clone();
        tokio::spawn(async move {
            leaderboard.run(srv_leaderboard).await;
        });

        let npcs = Arc::clone(&self.state.npcs);
        let auth = Arc::clone(&self.state.auth);
        let srv_npcs = server.clone();
        tokio::spawn(async move {
            npcs.run_background_loops(srv_npcs, auth).await;
        });

        if let Some(heartbeat) = &self.state.heartbeat {
            Arc::clone(heartbeat).spawn_loop(server.clone());
        }

        let lobby_hud = self.clone();
        let srv_hud = server.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(HUD_REFRESH_INTERVAL);
            loop {
                ticker.tick().await;
                if let Some(orch) = &lobby_hud.state.orchestrator
                    && let Ok(snap) = orch.fetch_status().await
                {
                    let counts = QueueCounts::from_snapshot(&snap);
                    lobby_hud.state.menus.refresh_games_menu(counts);
                    lobby_hud.state.npcs.update_pedestal_labels(counts);
                    if let Ok(online) = srv_hud.get_players().await {
                        for game in GameKind::ALL {
                            let Some(fleet) = game_fleet(&snap, game) else {
                                continue;
                            };
                            let state_hint = if fleet.ready_servers > 0 {
                                format!("§b{} serveur(s) prêt(s)", fleet.ready_servers)
                            } else if fleet.starting_servers > 0 {
                                "§eDémarrage serveur…".to_owned()
                            } else {
                                format!("§7{}/{} max", fleet.in_game_servers, fleet.max_replicas)
                            };
                            for qp in &fleet.queued_players {
                                if let Some(p) = online.iter().find(|p| p.uuid == qp.player_uuid) {
                                    p.action_bar.send(format!(
                                        "{} §8• §aFile #{}/{}{} §8• {state_hint}",
                                        game_tag(game),
                                        qp.position,
                                        fleet.queue_len,
                                        queue_min_hint(game, fleet.min_players)
                                    ));
                                }
                            }
                        }
                    }
                }
                let metrics = lobby_hud.collect_metrics(&srv_hud).await;
                lobby_hud
                    .state
                    .hud
                    .refresh_all(
                        &srv_hud,
                        &lobby_hud.state.auth,
                        &lobby_hud.state.profiles,
                        metrics,
                    )
                    .await;
            }
        });

        tracing::info!(
            "Hypolia Lobby initialisé (FerrisKey={} realm={}, Capitainerie={:?} realm={}, Orchestrator={:?})",
            self.state.config.ferriskey_base_url,
            self.state.config.ferriskey_realm,
            self.state.config.capitainerie_url,
            self.state.config.capitainerie_realm,
            self.state.config.orchestrator_url,
        );
    }

    async fn announce_join(&self, event: &mut PlayerJoin) {
        let message = hypolia_protocol::chat::join_message(&event.player().name);
        event.set_join_message(Some(message));
    }

    async fn announce_quit(&self, event: &mut PlayerLeave) {
        let message = hypolia_protocol::chat::quit_message(&event.player().name);
        event.set_quit_message(Some(message));
    }

    async fn on_player_join(&self, event: &PlayerJoin) {
        let player = event.player().clone();
        let lobby = self.clone();

        lobby.state.auth.hydrate(player.uuid).await;

        player.game_mode.set(GameMode::Adventure);
        player.health.set(20.0);
        player.health.set_fire_ticks(0);
        player.food.set_level(20);
        player.food.set_saturation(20.0);
        player.inventory.clear();
        player.inventory.set_held_slot(0);

        let prefs = lobby.state.prefs.get(player.uuid);
        prefs.apply_movement(&player);

        let claims = lobby.state.auth.claims_of(player.uuid);
        apply_lobby_hotbar(&player, &prefs);

        player.title.send(
            Title::new("§6§lHYPOLIA")
                .subtitle(format!("§dBienvenue sur le Hub, §f{}", player.name)),
        );
        player.sound.play(
            SoundCue::new(Sound::EntityPlayerLevelup)
                .volume(0.75)
                .pitch(1.1),
        );

        match &claims {
            Some(user) => {
                player.message.send(format!(
                    "§6§lHypolia §8» §7Bon retour §a✔ {} §7! Utilisez la §eBoussole §7ou les §dPlots §7pour jouer.",
                    user.username
                ));
            }
            None => {
                player.message.send_rich(
                    RichMessage::new()
                        .text("§6§lHypolia §8» §7Bienvenue ! Liez votre compte personnel avec ")
                        .part(
                            Part::new("§b§l[/login]")
                                .hover("§eCliquez pour connecter votre compte FerrisKey")
                                .click(Click::RunCommand("/login".to_owned())),
                        )
                        .text(" §7pour débloquer votre grade et la Capitainerie."),
                );
            }
        }

        tokio::spawn(async move {
            let spawn = lobby.state.layout.snapshot().spawn;
            let _ = player.location.teleport_facing(spawn.facing()).await;

            let srv = player.server();
            if let Err(err) = lobby.state.hud.attach_player(&srv, &player).await {
                tracing::warn!(
                    "Impossible d'attacher le scoreboard de {}: {err}",
                    player.name
                );
            }

            // Respect existing players' visibility settings
            if let Ok(online) = srv.get_players().await {
                let hiding_set: HashSet<Uuid> = lobby
                    .state
                    .prefs
                    .viewers_hiding_others()
                    .into_iter()
                    .collect();
                for other in &online {
                    if other.uuid == player.uuid {
                        continue;
                    }
                    if hiding_set.contains(&other.uuid) {
                        other.visibility.hide(&player);
                    }
                    if !prefs.players_visible {
                        player.visibility.hide(other);
                    }
                }
            }

            lobby.refresh_single_player_ui(&player).await;
            lobby.sync_progression(&player).await;
        });
    }

    async fn on_player_leave(&self, event: &PlayerLeave) {
        let player = event.player();
        self.state.auth.cancel_pending(player.uuid);
        self.state.auth.evict(player.uuid);
        self.state.menus.discard_options(player.uuid);
        self.state.menus.discard_gadgets(player.uuid);
        self.state.menus.discard_progress(player.uuid);
        self.state.profiles.remove(player.uuid);
        self.state.gadgets.clear_player(player.uuid);
        self.state.hud.detach_player(player.uuid, &player.name);
        self.state.npcs.leave_captain_chat(player.uuid);
        self.state.chat.clear_player(player.uuid);
        self.state
            .last_steps
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player.uuid);
        self.state
            .portal_last_blast
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player.uuid);

        if let Some(orch) = self.state.orchestrator.clone() {
            let uuid = player.uuid;
            tokio::spawn(async move {
                let _ = orch.leave_if_queued(uuid).await;
            });
        }

        let new_len = {
            let mut q = self
                .state
                .sheepwars_queue
                .write()
                .unwrap_or_else(|e| e.into_inner());
            if q.remove(&player.uuid) {
                Some(q.len())
            } else {
                None
            }
        };
        if let Some(count) = new_len {
            let counts = QueueCounts::default().with(GameKind::SheepWars, count);
            self.state.menus.refresh_games_menu(counts);
            self.state.npcs.update_pedestal_labels(counts);
        }
    }

    async fn on_player_interact(&self, event: &mut PlayerInteract) {
        let player = event.player().clone();
        let prefs = self.state.prefs.get(player.uuid);
        let is_right_click = matches!(
            event.action(),
            Action::RightClickAir | Action::RightClickBlock
        );

        if !prefs.build_mode {
            event.set_use_interacted_block(EventResult::Deny);
            event.set_use_item_in_hand(EventResult::Deny);
        }

        if !is_right_click || event.hand() == Some(EquipmentSlot::OffHand) {
            return;
        }

        match event.material() {
            Material::Compass => {
                let lobby = self.clone();
                tokio::spawn(async move {
                    player
                        .sound
                        .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                    let _ = lobby.state.menus.open_games(&player).await;
                });
            }
            Material::PlayerHead if !prefs.build_mode => {
                player
                    .sound
                    .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                self.open_progress_for(player, ProgressMenuKind::Profile);
            }
            Material::Chest if !prefs.build_mode => {
                player
                    .sound
                    .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                self.open_gadgets_menu_for(player);
            }
            Material::LimeDye | Material::GrayDye if !prefs.build_mode => {
                self.toggle_visibility(player);
            }
            other if !prefs.build_mode => {
                if let Some(gadget) = LobbyGadget::from_material(&other)
                    && prefs.equipped_gadget == Some(gadget)
                {
                    if self
                        .state
                        .profiles
                        .with(player.uuid, |p| gadget_usable(p, gadget))
                    {
                        self.state.gadgets.activate(player, gadget);
                    } else {
                        player
                            .action_bar
                            .send(locked_cosmetic_message(gadget.cosmetic()));
                    }
                }
            }
            _ => {}
        }
    }

    // Locked custom menus (Games, Options, Gadgets) are refused by rspigot and dispatched
    // only to Observe (`&InventoryClick`) handlers in Phase::Settled.
    async fn on_menu_click(&self, event: &InventoryClick) {
        self.handle_menu_click(
            event.player().clone(),
            event.clicked_inventory(),
            event.slot(),
        );
    }

    async fn on_inventory_click(&self, event: &mut InventoryClick) {
        let player = event.player();
        // Prevent moving hotbar items if top view is a custom menu or player is not in build mode
        let top_is_custom = event.inventory().custom().is_some();
        if top_is_custom || !self.state.prefs.get(player.uuid).build_mode {
            event.cancel();
        }
    }

    async fn on_inventory_close(&self, event: &InventoryClose) {
        self.state
            .menus
            .handle_close(event.player().uuid, event.inventory());
    }

    async fn on_player_interact_at_entity(&self, event: &PlayerInteractAtEntity) {
        if event.hand() != EquipmentSlot::Hand {
            return;
        }
        let player = event.player().clone();
        if self
            .state
            .leaderboard
            .is_hitbox(event.right_clicked().entity_id)
            .await
        {
            let game = self
                .state
                .leaderboard
                .switch_game(&player.server().world)
                .await;
            let pitch = match game {
                GameKind::SheepWars => 1.0,
                GameKind::TowerWars => 1.4,
            };
            player
                .sound
                .play(SoundCue::new(Sound::UiButtonClick).volume(0.7).pitch(pitch));
            player
                .action_bar
                .send(format!("§7Classement : {}", game_tag(game)));
            return;
        }
        let Some(target) = self
            .state
            .npcs
            .lookup_entity(event.right_clicked().entity_id)
        else {
            return;
        };

        match target {
            NpcClickTarget::Pedestal(spec) => {
                if let Some(game) = pedestal_game(&spec) {
                    if game == GameKind::TowerWars {
                        self.join_towerwars(player);
                    } else {
                        self.join_game(player, game, 0);
                    }
                } else {
                    player.sound.play(
                        SoundCue::new(Sound::BlockNoteBlockBell)
                            .volume(0.7)
                            .pitch(0.9),
                    );
                    player.message.send(format!(
                        "{} §8» §7Ce mini-jeu est en préparation et ouvrira prochainement !",
                        spec.title
                    ));
                }
            }
            NpcClickTarget::Ambient { npc, spec, line } => {
                npc.look_at(player.position);
                npc.swing_main_hand();
                player.sound.play(
                    SoundCue::new(Sound::EntityVillagerYes)
                        .volume(0.8)
                        .pitch(1.05),
                );
                player
                    .message
                    .send(format!("{} §8» §f{line}", spec.display_name));
            }
            NpcClickTarget::Agent(agent) => {
                if !self.state.auth.is_capitainerie_authenticated(player.uuid) {
                    self.prompt_capitainerie_login_required(
                        &player,
                        &format!("l'agent {}", agent.name),
                    );
                    return;
                }
                if agent.paused {
                    player
                        .message
                        .send(format!("§c{} est actuellement en pause.", agent.name));
                    return;
                }
                let intro = agent.role.clone().unwrap_or_else(|| {
                    "Confiez une mission avec votre compte FerrisKey.".to_owned()
                });
                player.dialog.show(
                    Dialog::new(format!("§b§lMission pour {}", agent.name))
                        .line(intro)
                        .input(DialogInput::new("title", "Titre").max_length(MAX_TASK_TITLE))
                        .input(
                            DialogInput::new("description", "Description")
                                .multiline()
                                .max_length(MAX_TASK_DESC),
                        )
                        .button(DialogButton::new(
                            "§aLancer la mission",
                            format!("{RUN_BUTTON_PREFIX}{}", click_slug(&agent.slug)),
                        ))
                        .button(DialogButton::new("§cAnnuler", CANCEL_BUTTON_ID)),
                );
            }
            NpcClickTarget::Captain => {
                if !self.state.npcs.has_capitainerie() {
                    player.message.send(
                        "§6Capitaine §8» §7La passerelle Capitainerie n'est pas configurée sur ce lobby (CAPITAINERIE_URL).",
                    );
                    return;
                }
                if !self.state.auth.is_capitainerie_authenticated(player.uuid) {
                    self.prompt_capitainerie_login_required(&player, CAPTAIN_DISPLAY_NAME);
                    return;
                }
                if self.state.npcs.toggle_captain_chat(player.uuid) {
                    player.sound.play(SoundCue::new(Sound::BlockNoteBlockChime));
                    player.message.send(format!(
                        "§6§l{CAPTAIN_DISPLAY_NAME} §8» §aCanal privé ouvert sous votre identité FerrisKey. §7Écrivez dans le tchat ou tapez §fquitter§7."
                    ));
                } else {
                    player
                        .message
                        .send("§6Capitaine §8» §7Canal privé fermé, retour au tchat général.");
                }
            }
        }
    }

    async fn on_player_custom_click(&self, event: &PlayerCustomClick) {
        let click_id = event.click_id();
        let player = event.player().clone();

        if click_id == CANCEL_BUTTON_ID {
            return;
        }
        if click_id == LOGIN_BUTTON_ID {
            self.start_ferriskey_login(player);
            return;
        }
        if click_id == CAP_LOGIN_BUTTON_ID {
            self.start_capitainerie_login(player);
            return;
        }

        if let Some(key) = click_id.strip_prefix(RESULT_BUTTON_PREFIX) {
            let report = self
                .state
                .task_results
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .get(key)
                .cloned();
            if let Some(report) = report {
                player
                    .sound
                    .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
                player.dialog.show(build_run_result_dialog(&report));
            } else {
                player.message.send(
                    "§eCe rapport n'est plus en cache. Consultez-le directement sur Capitainerie.",
                );
            }
            return;
        }

        let Some(slug) = click_id.strip_prefix(RUN_BUTTON_PREFIX) else {
            return;
        };
        let Some(agent) = self.state.npcs.agent_by_click_slug(slug) else {
            player.message.send("§cCet agent n'est plus disponible.");
            return;
        };

        let payload: serde_json::Value = event
            .data()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_default();
        let title = json_field(&payload, "title");
        let description = json_field(&payload, "description");
        if title.is_empty() {
            player
                .message
                .send("§cVeuillez indiquer un titre pour lancer la mission.");
            return;
        }

        let lobby = self.clone();
        tokio::spawn(async move {
            let bearer = match lobby.state.auth.valid_capitainerie_token(player.uuid).await {
                Ok(token) => token,
                Err(err) => {
                    player.message.send(format!("§c{err}"));
                    return;
                }
            };

            match lobby
                .state
                .npcs
                .start_agent_task(&bearer, &agent, &title, &description)
                .await
            {
                Ok(started) => {
                    player
                        .sound
                        .play(SoundCue::new(Sound::BlockAnvilUse).volume(0.5).pitch(1.3));
                    player.message.send(format!(
                        "§a✔ {} rejoint son établi pour traiter : §f{title}",
                        agent.name
                    ));
                    lobby.spawn_run_watcher(player, agent, title, started);
                }
                Err(err) => {
                    tracing::warn!("Échec de lancement de tâche sur {}: {err}", agent.slug);
                    player.message.send(err.explain());
                }
            }
        });
    }

    async fn on_async_player_chat(&self, event: &mut AsyncPlayerChat) {
        event.cancel();
        let player = event.player().clone();
        let text = event.message().trim().to_owned();
        if text.is_empty() {
            return;
        }

        // Priority 1: Private SSE conversation with the Captain
        if self.state.npcs.is_in_captain_chat(player.uuid) {
            if NpcDirector::is_quit_word(&text) {
                self.state.npcs.leave_captain_chat(player.uuid);
                player
                    .message
                    .send("§6Capitaine §8» §7Conversation terminée, retour au tchat du Hub.");
                return;
            }
            let conversation = match self.state.npcs.begin_captain_turn(player.uuid) {
                CaptainTurn::Busy => {
                    player.message.send(format!(
                        "§7Le {CAPTAIN_DISPLAY_NAME} rédige encore sa réponse, patientez…"
                    ));
                    return;
                }
                CaptainTurn::Ready(conv) => conv,
            };

            player
                .message
                .send(format!("§bVous §8→ §6Capitaine §8» §f{text}"));
            let lobby = self.clone();
            tokio::spawn(async move {
                let bearer = match lobby.state.auth.valid_capitainerie_token(player.uuid).await {
                    Ok(token) => token,
                    Err(err) => {
                        lobby.state.npcs.leave_captain_chat(player.uuid);
                        player.message.send(format!("§c{err}"));
                        return;
                    }
                };

                match lobby
                    .state
                    .npcs
                    .chat_with_captain(&bearer, &player, conversation.as_deref(), &text)
                    .await
                {
                    Ok(outcome) => {
                        for (_, card_title) in &outcome.cards {
                            player
                                .message
                                .send(format!("§a+ Carte créée : §f{card_title}"));
                        }
                        lobby
                            .state
                            .npcs
                            .end_captain_turn(player.uuid, Some(outcome.conversation));
                    }
                    Err(err) => {
                        tracing::warn!("Erreur conversation Capitaine : {err}");
                        player.message.send(err.explain());
                        lobby.state.npcs.end_captain_turn(player.uuid, None);
                    }
                }
            });
            return;
        }

        // Priority 2: Formatted Lobby Chat with FerrisKey Badges & @Mentions
        let claims = self.state.auth.claims_of(player.uuid);
        let has_admin = player.permissions.has("hypolia.admin");
        let prelim_rank = Rank::resolve(claims.as_ref(), false, has_admin);
        if let Err(wait) = self.state.chat.check_cooldown(player.uuid, prelim_rank) {
            player.message.send(format!(
                "§cVeuillez patienter {:.1}s avant d'envoyer un nouveau message.",
                wait.as_secs_f32()
            ));
            return;
        }

        let lobby = self.clone();
        tokio::spawn(async move {
            let srv = player.server();
            let is_op = player.get_state().await.is_ok_and(|s| s.op);
            let rank = Rank::resolve(claims.as_ref(), is_op, has_admin);
            let Ok(recipients) = srv.get_players().await else {
                return;
            };
            lobby.state.chat.broadcast_chat(
                &player,
                ChatBadge {
                    rank,
                    level: lobby.state.profiles.level(player.uuid),
                },
                claims.as_ref(),
                &text,
                &recipients,
                &lobby.state.prefs,
            );
        });
    }

    async fn on_block_break(&self, event: &mut BlockBreak) {
        if !self.state.prefs.get(event.player().uuid).build_mode {
            event.cancel();
        }
    }

    async fn on_block_place(&self, event: &mut BlockPlace) {
        if !self.state.prefs.get(event.player().uuid).build_mode {
            event.cancel();
        }
    }

    async fn on_player_drop_item(&self, event: &mut PlayerDropItem) {
        if !self.state.prefs.get(event.player().uuid).build_mode {
            event.cancel();
        }
    }

    async fn on_player_swap_hand_items(&self, event: &mut PlayerSwapHandItems) {
        if !self.state.prefs.get(event.player().uuid).build_mode {
            event.cancel();
        }
    }

    async fn on_entity_damage(&self, event: &mut EntityDamage) {
        event.cancel();
    }

    async fn on_food_level_change(&self, event: &mut FoodLevelChange) {
        event.cancel();
    }

    async fn on_leaves_decay(&self, event: &mut LeavesDecay) {
        event.cancel();
    }

    async fn on_block_fade(&self, event: &mut BlockFade) {
        event.cancel();
    }

    async fn on_block_burn(&self, event: &mut BlockBurn) {
        event.cancel();
    }

    async fn on_block_spread(&self, event: &mut BlockSpread) {
        event.cancel();
    }

    async fn on_block_grow(&self, event: &mut BlockGrow) {
        event.cancel();
    }

    async fn on_moisture_change(&self, event: &mut MoistureChange) {
        event.cancel();
    }

    async fn on_player_move(&self, event: &mut PlayerMove) {
        let Some(to) = event.to() else { return };
        let layout = self.state.layout.snapshot();

        // Rescue player if they jump off the floating island
        if to.position.y < layout.void_y {
            let player = event.player().clone();
            let spawn: Facing = layout.spawn.facing();
            tokio::spawn(async move {
                if player.location.teleport_facing(spawn).await.is_ok() {
                    player
                        .sound
                        .play(SoundCue::new(Sound::EntityEndermanTeleport));
                    player.action_bar.send("§e✦ Retour sur l'île de Hypolia !");
                }
            });
            return;
        }

        let from = event.from();
        let step = Vector {
            x: to.position.x - from.position.x,
            y: 0.0,
            z: to.position.z - from.position.z,
        };
        self.state
            .last_steps
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(event.player().uuid, step);

        let prefs = self.state.prefs.get(event.player().uuid);
        if prefs.aura_trail != AuraTrail::None
            && self
                .state
                .profiles
                .with(event.player().uuid, |p| trail_usable(p, prefs.aura_trail))
        {
            self.state
                .gadgets
                .emit_trail_if_due(event.player(), prefs.aura_trail);
        }
    }

    async fn on_player_portal(&self, event: &mut PlayerPortal) {
        if event.cause() == PlayerTeleportCause::EndPortal {
            event.cancel();
        }
    }

    async fn on_entity_portal_enter(&self, event: &EntityPortalEnter) {
        if event.entity_type() != EntityType::Player || self.state.orchestrator.is_none() {
            return;
        }
        let entity = event.entity();
        let uuid = entity.uuid;
        let now = Instant::now();
        let debounced = {
            let mut guard = self
                .state
                .portal_last_blast
                .write()
                .unwrap_or_else(|e| e.into_inner());
            match guard.get(&uuid) {
                Some(last) if now.saturating_duration_since(*last) < PORTAL_DEBOUNCE => true,
                _ => {
                    guard.insert(uuid, now);
                    false
                }
            }
        };
        if debounced {
            return;
        }
        let step = self
            .state
            .last_steps
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&uuid)
            .copied();
        let at = entity.position;
        let server = entity.server();
        let lobby = self.clone();
        tokio::spawn(async move {
            let Ok(players) = server.get_players().await else {
                return;
            };
            if let Some(player) = players.into_iter().find(|p| p.uuid == uuid) {
                lobby.portal_blast(player, at, step, 0.0);
            }
        });
    }

    #[command("menu")]
    async fn cmd_menu(&self, ctx: Context) {
        let player = ctx.sender.clone();
        player
            .sound
            .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
        if let Err(err) = self.state.menus.open_games(&player).await {
            ctx.reply(format!("§cImpossible d'ouvrir le menu : {err}"));
        }
    }

    #[command("servers")]
    async fn cmd_servers(&self, ctx: Context) {
        let player = ctx.sender.clone();
        if !self.is_staff(&player).await {
            ctx.reply("§cRéservé au staff.");
            return;
        }
        let Some(client) = &self.state.orchestrator else {
            ctx.reply("§cOrchestrateur non configuré.");
            return;
        };
        let snapshot = match client.fetch_status().await {
            Ok(snapshot) => snapshot,
            Err(err) => {
                ctx.reply(format!("§cImpossible de joindre l'orchestrateur : {err}"));
                return;
            }
        };
        player
            .sound
            .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
        if let Err(err) = self
            .state
            .menus
            .open_servers(
                &player.server(),
                &player,
                self.state.lobby_version.as_deref(),
                &snapshot,
            )
            .await
        {
            ctx.reply(format!("§cImpossible d'ouvrir le menu : {err}"));
        }
    }

    #[command("options")]
    async fn cmd_options(&self, ctx: Context) {
        let player = ctx.sender.clone();
        let prefs = self.state.prefs.get(player.uuid);
        if let Err(err) = self
            .state
            .menus
            .open_options(&player.server(), &player, &prefs)
            .await
        {
            ctx.reply(format!("§cImpossible d'ouvrir vos options : {err}"));
        }
    }

    #[command("gadgets")]
    async fn cmd_gadgets(&self, ctx: Context) {
        let player = ctx.sender.clone();
        player
            .sound
            .play(SoundCue::new(Sound::UiButtonClick).volume(0.7));
        let prefs = self.state.prefs.get(player.uuid);
        let profile = self.state.profiles.snapshot(player.uuid);
        if let Err(err) = self
            .state
            .menus
            .open_gadgets(&player.server(), &player, &prefs, profile.as_ref())
            .await
        {
            ctx.reply(format!("§cImpossible d'ouvrir les gadgets : {err}"));
        }
    }

    #[command("profil")]
    async fn cmd_profil(&self, ctx: Context) {
        self.open_progress_for(ctx.sender.clone(), ProgressMenuKind::Profile);
    }

    #[command("boutique")]
    async fn cmd_boutique(&self, ctx: Context) {
        self.open_progress_for(
            ctx.sender.clone(),
            ProgressMenuKind::Shop(CosmeticSlot::LobbyGadget),
        );
    }

    #[command("coins")]
    async fn cmd_coins(
        &self,
        ctx: Context,
        action: Option<String>,
        target: Option<Player>,
        amount: Option<u64>,
    ) {
        let sender = ctx.sender.clone();
        let claims = self.state.auth.claims_of(sender.uuid);
        let is_op = sender.get_state().await.is_ok_and(|s| s.op);
        let has_admin = sender.permissions.has("hypolia.admin");
        if !can_grant_coins(Rank::resolve(claims.as_ref(), is_op, has_admin)) {
            ctx.reply("§cRéservé aux admins.");
            return;
        }
        let (Some("give"), Some(target), Some(amount)) = (action.as_deref(), target, amount) else {
            ctx.reply("§cUsage : /coins give <joueur> <montant>");
            return;
        };
        if amount == 0 {
            ctx.reply("§cLe montant doit être supérieur à 0.");
            return;
        }
        let Some(client) = &self.state.orchestrator else {
            ctx.reply("§cOrchestrateur non configuré.");
            return;
        };
        let grant = AdminCoinGrant {
            amount,
            admin: sender.name.clone(),
        };
        match client.grant_coins(target.uuid, &grant).await {
            Ok(profile) => {
                self.store_profile(target.uuid, profile, false);
                ctx.reply(format!(
                    "§a✔ +{amount} coins crédités à §f{}§a.",
                    target.name
                ));
                target.message.send(format!(
                    "§6§lHypolia §8» §a+{amount} coins §7(crédit de §f{}§7)",
                    sender.name
                ));
                self.refresh_single_player_ui(&target).await;
            }
            Err(err) => ctx.reply(error_message(&err)),
        }
    }

    #[command("fly")]
    async fn cmd_fly(&self, ctx: Context) {
        self.toggle_flight_for(&ctx.sender);
    }

    #[command("speed")]
    async fn cmd_speed(&self, ctx: Context) {
        self.cycle_speed_for(&ctx.sender);
    }

    #[command("spawn")]
    async fn cmd_spawn(&self, ctx: Context) {
        self.teleport_to_poi(ctx.sender.clone(), "spawn");
    }

    #[command("towerwars")]
    async fn cmd_towerwars(&self, ctx: Context) {
        self.join_towerwars(ctx.sender.clone());
    }

    #[command("deacoudre")]
    async fn cmd_deacoudre(&self, ctx: Context) {
        self.join_deacoudre(ctx.sender.clone());
    }

    #[command("sheepwars")]
    async fn cmd_sheepwars(&self, ctx: Context) {
        self.join_sheepwars(ctx.sender.clone());
    }

    #[command("login")]
    async fn cmd_login(&self, ctx: Context) {
        let player = ctx.sender.clone();
        if let Some(claims) = self.state.auth.claims_of(player.uuid) {
            ctx.reply(format!(
                "§a✔ Vous êtes déjà connecté en tant que §f{}§a. Utilisez §e/logout §apour changer de compte.",
                claims.username
            ));
            return;
        }
        self.start_ferriskey_login(player);
    }

    #[command("logout")]
    async fn cmd_logout(&self, ctx: Context) {
        let player = ctx.sender.clone();
        if self.state.auth.logout(player.uuid).await {
            if !self.state.auth.uses_separate_capitainerie_auth() {
                self.state.npcs.leave_captain_chat(player.uuid);
            }
            ctx.reply("§eVotre compte FerrisKey a été déconnecté.");
            self.refresh_single_player_ui(&player).await;
            self.sync_progression(&player).await;
        } else {
            ctx.reply("§7Aucun compte FerrisKey n'était actif.");
        }
    }

    #[command("caplogin")]
    async fn cmd_caplogin(&self, ctx: Context) {
        let player = ctx.sender.clone();
        if !self.state.npcs.has_capitainerie() {
            ctx.reply("§7La passerelle Capitainerie n'est pas configurée sur ce lobby (CAPITAINERIE_URL).");
            return;
        }
        if !self.state.auth.uses_separate_capitainerie_auth() {
            if let Some(claims) = self.state.auth.claims_of(player.uuid) {
                ctx.reply(format!(
                    "§a✔ Capitainerie partage votre compte FerrisKey (§f{}§a).",
                    claims.username
                ));
            } else {
                self.start_ferriskey_login(player);
            }
            return;
        }
        if let Some(claims) = self.state.auth.capitainerie_claims_of(player.uuid) {
            ctx.reply(format!(
                "§a✔ Déjà connecté à Capitainerie (realm §b{}§a) en tant que §f{}§a. Utilisez §e/caplogout §apour changer de compte.",
                self.state.config.capitainerie_realm,
                claims.username
            ));
            return;
        }
        self.start_capitainerie_login(player);
    }

    #[command("caplogout")]
    async fn cmd_caplogout(&self, ctx: Context) {
        let player = ctx.sender.clone();
        if !self.state.auth.uses_separate_capitainerie_auth() {
            ctx.reply("§7Capitainerie utilise la même session que le Lobby (/logout).");
            return;
        }
        if self.state.auth.capitainerie_logout(player.uuid).await {
            self.state.npcs.leave_captain_chat(player.uuid);
            ctx.reply("§eVotre compte Capitainerie a été déconnecté.");
        } else {
            ctx.reply("§7Aucun compte Capitainerie n'était actif.");
        }
    }

    #[command("whoami")]
    async fn cmd_whoami(&self, ctx: Context) {
        self.show_whoami(&ctx.sender);
    }

    #[command("mp")]
    async fn cmd_mp(&self, ctx: Context, target: Player, message: String) {
        let sender = &ctx.sender;
        let body = message.trim();
        if body.is_empty() {
            ctx.reply("§cMessage vide.");
            return;
        }
        if target.uuid == sender.uuid {
            ctx.reply("§eVous ne pouvez pas vous écrire à vous-même.");
            return;
        }
        self.state
            .chat
            .send_whisper(sender, &target, body, &self.state.prefs);
    }

    #[command("r")]
    async fn cmd_reply(&self, ctx: Context, message: String) {
        let sender = ctx.sender.clone();
        let body = message.trim();
        if body.is_empty() {
            ctx.reply("§cMessage vide.");
            return;
        }
        let Some(partner_id) = self.state.chat.reply_partner(sender.uuid) else {
            ctx.reply("§cPersonne à qui répondre.");
            return;
        };
        let Ok(online) = sender.server().get_players().await else {
            return;
        };
        let Some(target) = online.into_iter().find(|p| p.uuid == partner_id) else {
            ctx.reply("§cCe joueur n'est plus connecté.");
            return;
        };
        self.state
            .chat
            .send_whisper(&sender, &target, body, &self.state.prefs);
    }

    #[command("build", permission = "hypolia.admin")]
    async fn cmd_build(&self, ctx: Context) {
        let player = &ctx.sender;
        let updated = self
            .state
            .prefs
            .update(player.uuid, |p| p.build_mode = !p.build_mode);
        if updated.build_mode {
            player.game_mode.set(GameMode::Creative);
            ctx.reply("§a🛠 Mode construction du Lobby ACTIVÉ.");
        } else {
            player.game_mode.set(GameMode::Adventure);
            updated.apply_movement(player);
            apply_lobby_hotbar(player, &updated);
            ctx.reply("§e🛠 Mode construction du Lobby DÉSACTIVÉ.");
        }
    }

    #[command("lobby", permission = "hypolia.admin")]
    async fn cmd_lobby(
        &self,
        ctx: Context,
        #[suggest(Self::suggest_lobby_subcommands)] action: String,
        arg1: Option<String>,
        arg2: Option<usize>,
    ) {
        let player = ctx.sender.clone();
        let pose = WaypointPose::from_player(&player);

        match action.as_str() {
            "info" => {
                let snap = self.state.layout.snapshot();
                ctx.reply(format!(
                    "§6§lLobby §8» §7Spawn=({:.1}, {:.1}, {:.1}) | Plots={} | PNJ Village={} | Postes Capitainerie={}",
                    snap.spawn.x,
                    snap.spawn.y,
                    snap.spawn.z,
                    snap.pedestals.len(),
                    snap.ambient_npcs.len(),
                    snap.agent_workstations.len()
                ));
            }
            "reload" => match self.state.layout.reload().await {
                Ok(_) => {
                    self.state
                        .npcs
                        .spawn_all(&player.server(), self.queued_counts())
                        .await;
                    self.state.leaderboard.sync(&player.server().world).await;
                    ctx.reply("§a✔ Configuration lobby.json rechargée et PNJ synchronisés.");
                }
                Err(err) => ctx.reply(format!("§cErreur de rechargement : {err}")),
            },
            "setspawn" => {
                let res = self
                    .state
                    .layout
                    .update(|l| {
                        l.spawn = pose;
                        l.pois.insert("spawn".to_owned(), pose);
                        Ok(())
                    })
                    .await;
                match res {
                    Ok(_) => ctx.reply("§a✔ Point de Spawn mis à jour sur votre position."),
                    Err(err) => ctx.reply(format!("§c{err}")),
                }
            }
            "setpoi" => {
                let Some(name) = arg1 else {
                    ctx.reply("§eUsage : /lobby setpoi <spawn|plaza|portal|capitainerie|castle>");
                    return;
                };
                let res = self
                    .state
                    .layout
                    .update(|l| {
                        l.pois.insert(name.clone(), pose);
                        Ok(())
                    })
                    .await;
                match res {
                    Ok(_) => ctx.reply(format!("§a✔ Point d'intérêt '{name}' enregistré.")),
                    Err(err) => ctx.reply(format!("§c{err}")),
                }
            }
            "setpedestal" => {
                let Some(idx) = arg1.as_deref().and_then(|s| s.parse::<usize>().ok()) else {
                    ctx.reply("§eUsage : /lobby setpedestal <1..7>");
                    return;
                };
                let res = self
                    .state
                    .layout
                    .update(|l| {
                        let slot = idx
                            .checked_sub(1)
                            .and_then(|i| l.pedestals.get_mut(i))
                            .ok_or_else(|| {
                                ConfigError::OutOfBounds(format!("plot {idx} invalide (1..=7)"))
                            })?;
                        slot.pose = pose;
                        Ok(())
                    })
                    .await;
                match res {
                    Ok(_) => {
                        self.state
                            .npcs
                            .spawn_all(&player.server(), self.queued_counts())
                            .await;
                        ctx.reply(format!("§a✔ Plot #{idx} repositionné et PNJ mis à jour."));
                    }
                    Err(err) => ctx.reply(format!("§c{err}")),
                }
            }
            "setleaderboard" => {
                let res = self
                    .state
                    .layout
                    .update(|l| {
                        l.leaderboard = Some(pose);
                        Ok(())
                    })
                    .await;
                match res {
                    Ok(_) => {
                        self.state.leaderboard.sync(&player.server().world).await;
                        ctx.reply("§a✔ Tableau des meilleurs joueurs placé sur votre position.");
                    }
                    Err(err) => ctx.reply(format!("§c{err}")),
                }
            }
            "setcaptain" => {
                let res = self
                    .state
                    .layout
                    .update(|l| {
                        l.captain = pose;
                        Ok(())
                    })
                    .await;
                match res {
                    Ok(_) => {
                        self.state
                            .npcs
                            .spawn_all(&player.server(), self.queued_counts())
                            .await;
                        ctx.reply("§a✔ Position du Capitaine enregistrée.");
                    }
                    Err(err) => ctx.reply(format!("§c{err}")),
                }
            }
            "setstation" => {
                let Some(idx) = arg1.as_deref().and_then(|s| s.parse::<usize>().ok()) else {
                    ctx.reply("§eUsage : /lobby setstation <index_0_based>");
                    return;
                };
                let res = self
                    .state
                    .layout
                    .update(|l| {
                        if idx < l.agent_workstations.len() {
                            l.agent_workstations[idx] = pose;
                        } else if idx == l.agent_workstations.len() {
                            l.agent_workstations.push(pose);
                        } else {
                            return Err(ConfigError::OutOfBounds(format!(
                                "poste {idx} hors limites"
                            )));
                        }
                        Ok(())
                    })
                    .await;
                match res {
                    Ok(_) => ctx.reply(format!(
                        "§a✔ Poste de travail Capitainerie #{idx} enregistré."
                    )),
                    Err(err) => ctx.reply(format!("§c{err}")),
                }
            }
            "setwaypoint" => {
                let (Some(npc_id), Some(wp_idx)) = (arg1, arg2) else {
                    ctx.reply(
                        "§eUsage : /lobby setwaypoint <guard|lumberjack|mage|innkeeper> <index>",
                    );
                    return;
                };
                let res = self
                    .state
                    .layout
                    .update(|l| {
                        let npc = l
                            .ambient_npcs
                            .iter_mut()
                            .find(|n| n.id == npc_id)
                            .ok_or_else(|| {
                                ConfigError::OutOfBounds(format!("PNJ '{npc_id}' introuvable"))
                            })?;
                        if wp_idx < npc.waypoints.len() {
                            npc.waypoints[wp_idx] = pose;
                        } else if wp_idx == npc.waypoints.len() {
                            npc.waypoints.push(pose);
                        } else {
                            return Err(ConfigError::OutOfBounds(format!(
                                "waypoint {wp_idx} hors limites"
                            )));
                        }
                        Ok(())
                    })
                    .await;
                match res {
                    Ok(_) => {
                        self.state
                            .npcs
                            .spawn_all(&player.server(), self.queued_counts())
                            .await;
                        ctx.reply(format!(
                            "§a✔ Waypoint #{wp_idx} du PNJ '{npc_id}' enregistré."
                        ));
                    }
                    Err(err) => ctx.reply(format!("§c{err}")),
                }
            }
            other => {
                ctx.reply(format!(
                    "§eSous-commande '{other}' inconnue. Choix : {}",
                    LOBBY_SUBCOMMANDS.join(", ")
                ));
            }
        }
    }
}

fn portal_launch_velocity(entry_step: Option<Vector>, yaw: f32) -> Vector {
    let (dx, dz, len) = match entry_step {
        Some(step) => {
            let len = step.x.hypot(step.z);
            (step.x, step.z, len)
        }
        None => (0.0, 0.0, 0.0),
    };
    let (dir_x, dir_z, speed) = if len > 1e-3 {
        (
            -dx / len,
            -dz / len,
            (len * 4.0).clamp(PORTAL_LAUNCH_MIN_SPEED, PORTAL_LAUNCH_MAX_SPEED),
        )
    } else {
        let rad = f64::from(yaw.to_radians());
        (rad.sin(), -rad.cos(), PORTAL_LAUNCH_MIN_SPEED)
    };
    Vector {
        x: dir_x * speed,
        y: PORTAL_LAUNCH_LIFT,
        z: dir_z * speed,
    }
}

fn not_party_leader(tag: &str, leader: Option<&str>) -> String {
    let leader = leader.unwrap_or("le chef");
    format!(
        "{tag} §8» §cSeul §f{leader} §cpeut lancer une partie pour le groupe. §7/group leave pour jouer seul."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn towerwars_queue_hint_names_the_chosen_size() {
        let hint = |min, queued| queue_size_hint(GameKind::TowerWars, min, queued);
        assert_eq!(hint(2, 1), " §b(Solo — 1/2)§a");
        assert_eq!(hint(2, 5), " §b(Solo — 2/2)§a");
        assert_eq!(hint(4, 3), " §b(Duo — 3/4)§a");
        assert_eq!(hint(4, 9), " §b(Duo — 4/4)§a");
        assert_eq!(hint(0, 2), "");
        assert_eq!(hint(3, 2), "");
        assert_eq!(hint(8, 2), "");
    }

    #[test]
    fn sheepwars_keeps_the_legacy_min_hint_and_no_size() {
        assert_eq!(queue_size_hint(GameKind::SheepWars, 2, 1), "");
        assert_eq!(queue_min_hint(GameKind::SheepWars, 2), " §7(min §f2§7)");
        assert_eq!(queue_min_hint(GameKind::TowerWars, 2), "");
    }
}
