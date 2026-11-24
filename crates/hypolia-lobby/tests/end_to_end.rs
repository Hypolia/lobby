use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Form, Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use crossbeam_channel::{Receiver, RecvTimeoutError};
use rspigot_core::config::{GameConfig, LevelData};
use rspigot_core::events::{
    ClickInput, ClientEvent, ContainerClick, EntityKind, Hand, HashedSlot, Inbound, MetadataValue,
    PlayerInput,
};
use rspigot_core::ports::{ChunkGenerator, ChunkStore, ClientLink, EntityStore, StoreError};
use rspigot_core::server::Server;
use rspigot_core::status::StatusBoard;
use rspigot_domain::chunk::ChunkColumn;
use rspigot_domain::player::{ClientSettings, GameProfile, PlayerId};
use rspigot_domain::text::Component;
use rspigot_domain::world::Dimension;
use rspigot_domain::{
    BiomeId, BlockPos, BlockStateId, ChunkPos, EntityId, Identifier, Rotation, Vec3,
};
use rspigot_nbt::Compound;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::watch;

const TICK: Duration = Duration::from_millis(50);
const PATIENCE: Duration = Duration::from_secs(25);
const SPAWN: Vec3 = Vec3::new(13.5, 29.0, -82.5);

#[derive(Clone, Default)]
struct Link {
    events: Arc<Mutex<Vec<ClientEvent>>>,
    closed: Arc<AtomicBool>,
}

impl Link {
    fn take(&self) -> Vec<ClientEvent> {
        self.events
            .lock()
            .map(|mut e| std::mem::take(&mut *e))
            .unwrap_or_default()
    }
}

impl ClientLink for Link {
    fn send(&self, event: ClientEvent) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }

    fn disconnect(&self, _: Component) {
        self.closed.store(true, Ordering::Release);
    }

    fn is_connected(&self) -> bool {
        !self.closed.load(Ordering::Acquire)
    }
}

struct NoStore;

impl ChunkStore for NoStore {
    fn load(&self, _: &Identifier, _: ChunkPos) -> Result<Option<ChunkColumn>, StoreError> {
        Ok(None)
    }
    fn save(&self, _: &Identifier, _: &ChunkColumn) -> Result<(), StoreError> {
        Ok(())
    }
    fn flush(&self) -> Result<(), StoreError> {
        Ok(())
    }
}

impl EntityStore for NoStore {
    fn load_entities(
        &self,
        _: &Identifier,
        _: ChunkPos,
    ) -> Result<Option<Vec<Compound>>, StoreError> {
        Ok(None)
    }
    fn save_entities(&self, _: &Identifier, _: ChunkPos, _: &[Compound]) -> Result<(), StoreError> {
        Ok(())
    }
}

struct FlatIsland;

impl ChunkGenerator for FlatIsland {
    fn generate(&self, dimension: &Dimension, pos: ChunkPos) -> ChunkColumn {
        let mut chunk = ChunkColumn::empty(pos, dimension.height, BiomeId(0));
        for x in 0..16 {
            for z in 0..16 {
                let wx = pos.min_block_x() + x;
                let wz = pos.min_block_z() + z;
                let top_y = if wz <= -78 {
                    28
                } else if wz <= -46 {
                    26
                } else if (8..=18).contains(&wx) {
                    21
                } else {
                    23
                };
                for y in 15..=top_y {
                    chunk.set_block(BlockPos::new(wx, y, wz), BlockStateId(1));
                }
            }
        }
        chunk
    }

    fn spawn_point(&self, _: &Dimension) -> Vec3 {
        SPAWN
    }
}

struct TestPlayer {
    id: PlayerId,
    name: String,
    link: Link,
    log: Vec<ClientEvent>,
    confirmed_cursor: usize,
    pos: Vec3,
}

impl TestPlayer {
    fn drain_into_log(&mut self) -> &[ClientEvent] {
        self.log.extend(self.link.take());
        &self.log
    }

    fn received_chunk(&mut self, pos: ChunkPos) -> bool {
        self.drain_into_log()
            .iter()
            .any(|event| matches!(event, ClientEvent::Chunk { chunk, .. } if chunk.pos() == pos))
    }

    fn npc_entity_and_pos(&mut self, name: &str) -> Option<(i32, Vec3)> {
        let log = self.drain_into_log();
        let uuid = log.iter().find_map(|event| match event {
            ClientEvent::PlayerInfo(entries) => entries
                .iter()
                .find(|entry| entry.profile.name == name)
                .map(|entry| entry.profile.id.0),
            _ => None,
        })?;
        log.iter().find_map(|event| match event {
            ClientEvent::SpawnEntity {
                entity,
                uuid: spawned,
                kind: EntityKind::Player,
                position,
                ..
            } if *spawned == uuid => Some((entity.0, *position)),
            _ => None,
        })
    }

    fn label_has(&mut self, text: &str) -> bool {
        self.drain_into_log().iter().any(|event| match event {
            ClientEvent::EntityMetadata { entries, .. } => entries.iter().any(|entry| {
                matches!(&entry.value, MetadataValue::Component(shown) if shown.plain_text().contains(text))
            }),
            _ => false,
        })
    }

    fn said(&mut self, text: &str) -> bool {
        self.drain_into_log().iter().any(|event| {
            matches!(event, ClientEvent::SystemChat { message, .. } if message.plain_text().contains(text))
        })
    }

    fn dialog_buttons(&mut self) -> Vec<String> {
        self.drain_into_log()
            .iter()
            .rev()
            .find_map(|event| match event {
                ClientEvent::ShowDialog(dialog) => Some(
                    dialog
                        .buttons
                        .iter()
                        .map(|button| button.id.to_string())
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn dialog_has(&mut self, text: &str) -> bool {
        self.drain_into_log()
            .iter()
            .rev()
            .find_map(|event| match event {
                ClientEvent::ShowDialog(dialog) => {
                    let debug_str = format!("{dialog:?}");
                    Some(debug_str.contains(text))
                }
                _ => None,
            })
            .unwrap_or(false)
    }

    fn event_has(&mut self, text: &str) -> bool {
        self.drain_into_log()
            .iter()
            .any(|event| format!("{event:?}").contains(text))
    }

    fn vanilla_chat_seen(&mut self) -> bool {
        self.drain_into_log()
            .iter()
            .any(|event| matches!(event, ClientEvent::PlayerChat { .. }))
    }

    fn log_cursor(&mut self) -> usize {
        self.drain_into_log().len()
    }

    fn latest_screen_after(&mut self, cursor: usize, title_part: &str) -> Option<(i32, i32)> {
        let log = self.drain_into_log();
        let window = log[cursor..].iter().rev().find_map(|event| match event {
            ClientEvent::OpenScreen { window, title, .. }
                if title.plain_text().contains(title_part) =>
            {
                Some(*window)
            }
            _ => None,
        })?;
        let state_id = log[cursor..].iter().rev().find_map(|event| match event {
            ClientEvent::ContainerContent {
                window: w,
                state_id,
                ..
            } if *w == window => Some(*state_id),
            _ => None,
        })?;
        Some((window, state_id))
    }

    fn abilities_after(
        &mut self,
        cursor: usize,
        pred: impl Fn(&rspigot_core::events::Abilities) -> bool,
    ) -> bool {
        self.drain_into_log()[cursor..]
            .iter()
            .any(|event| matches!(event, ClientEvent::Abilities(ab) if pred(ab)))
    }

    fn has_velocity_after(&mut self, cursor: usize) -> bool {
        self.drain_into_log()[cursor..]
            .iter()
            .any(|event| matches!(event, ClientEvent::EntityVelocity { .. }))
    }

    fn has_particle_after(&mut self, cursor: usize) -> bool {
        self.drain_into_log()[cursor..]
            .iter()
            .any(|event| matches!(event, ClientEvent::Particle(_)))
    }

    fn has_sheep_spawn_after(&mut self, cursor: usize) -> bool {
        self.drain_into_log()[cursor..].iter().any(|event| {
            matches!(
                event,
                ClientEvent::SpawnEntity {
                    kind: EntityKind::Other(_),
                    ..
                }
            )
        })
    }
}

fn build_server() -> Server<Link> {
    let config = GameConfig {
        view_distance: 6,
        spawn_chunk_radius: 5,
        chunk_workers: 2,
        spawn_protection: 0,
        ..GameConfig::default()
    };
    let mut server = Server::new(
        config,
        LevelData::default(),
        Arc::new(NoStore),
        Arc::new(FlatIsland),
        StatusBoard::new(20),
    );
    server.wait_for_spawn(Duration::from_secs(10));
    server
}

fn confirm_teleports(server: &mut Server<Link>, player: &mut TestPlayer) {
    player.drain_into_log();
    let mut updates = Vec::new();
    for event in &player.log[player.confirmed_cursor..] {
        if let ClientEvent::Teleport { id, position, .. } = event {
            updates.push((*id, *position));
        }
    }
    player.confirmed_cursor = player.log.len();
    for (id, pos) in updates {
        player.pos = pos;
        server.handle(Inbound::Input {
            player: player.id,
            input: PlayerInput::TeleportConfirm(id),
        });
    }
    server.deliver();
}

const ARRIVAL_TICKS: usize = 200;
const ARRIVAL_RADIUS: f64 = 2.2;
const STAND_OFFSET: f64 = 1.5;

fn move_close_to(core: &mut Core, player: &mut TestPlayer, target: Vec3) {
    let spot = Vec3::new(target.x + STAND_OFFSET, target.y, target.z);
    core.server.handle(Inbound::Console(format!(
        "tp {} {} {} {}",
        player.name, spot.x, spot.y, spot.z
    )));
    for _ in 0..ARRIVAL_TICKS {
        confirm_teleports(&mut core.server, player);
        let dx = target.x - player.pos.x;
        let dy = target.y - player.pos.y;
        let dz = target.z - player.pos.z;
        if (dx * dx + dy * dy + dz * dz).sqrt() <= ARRIVAL_RADIUS {
            return;
        }
        let waiting = core.inbox.len();
        let inbox = core.inbox.clone();
        for msg in inbox.try_iter().take(waiting) {
            core.handle(msg);
        }
        core.server.tick();
        std::thread::sleep(Duration::from_millis(15));
    }
    panic!("player never reached {target:?}, still at {:?}", player.pos);
}

fn join(server: &mut Server<Link>, name: &str) -> TestPlayer {
    let link = Link::default();
    let id = PlayerId::offline(name);
    server.handle(Inbound::Join {
        profile: GameProfile {
            id,
            name: name.to_owned(),
            properties: Vec::new(),
        },
        settings: ClientSettings::default(),
        link: link.clone(),
        address: std::net::Ipv4Addr::LOCALHOST.into(),
        session: 0,
    });
    server.deliver();
    server.tick();
    let mut player = TestPlayer {
        id,
        name: name.to_owned(),
        link,
        log: Vec::new(),
        confirmed_cursor: 0,
        pos: SPAWN,
    };
    assert!(player.received_chunk(SPAWN.chunk()));
    confirm_teleports(server, &mut player);
    server.handle(Inbound::Input {
        player: id,
        input: PlayerInput::Loaded,
    });
    player
}

struct PluginProcess(Child);

impl PluginProcess {
    fn start(host: SocketAddr, mock_http: SocketAddr, temp_dir: &Path) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_hypolia-lobby"))
            .current_dir(temp_dir)
            .env("RSPIGOT_URL", format!("http://{host}"))
            .env_remove("RSPIGOT_TOKEN")
            .env("FERRISKEY_URL", format!("http://{mock_http}"))
            .env("FERRISKEY_REALM", "hypolia")
            .env("FERRISKEY_CLIENT_ID", "hypolia-lobby")
            .env_remove("FERRISKEY_CLIENT_SECRET")
            .env("CAPITAINERIE_URL", format!("http://{mock_http}"))
            .env("CAPITAINERIE_CONSOLE_URL", format!("http://{mock_http}"))
            .env("CAPITAINERIE_RESOURCE", format!("http://{mock_http}"))
            .env_remove("CAPITAINERIE_REALM")
            .env_remove("CAPITAINERIE_CLIENT_ID")
            .env_remove("CAPITAINERIE_CLIENT_SECRET")
            .env_remove("CAPITAINERIE_FERRISKEY_URL")
            .env_remove("CAPITAINERIE_OBSERVER_TOKEN")
            .env_remove("SHEEPWARS_HOST")
            .env_remove("SHEEPWARS_PORT")
            .env_remove("SHEEPWARS_BACKEND")
            .env_remove("SHEEPWARS_WORLD")
            .env("LOBBY_LAYOUT_PATH", temp_dir.join("lobby.json"))
            .env("LOBBY_SESSIONS_PATH", temp_dir.join("sessions.json"))
            .spawn()
            .expect("hypolia-lobby starts");
        Self(child)
    }
}

impl Drop for PluginProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Core {
    server: Server<Link>,
    inbox: Receiver<Inbound<Link>>,
    plugin: Option<u64>,
    next_tick: Instant,
}

impl Core {
    fn new(inbox: Receiver<Inbound<Link>>) -> Self {
        Self {
            server: build_server(),
            inbox,
            plugin: None,
            next_tick: Instant::now(),
        }
    }

    fn run_until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(self) {
            let now = Instant::now();
            assert!(now < deadline, "timed out waiting for: {what}");
            if now >= self.next_tick {
                let waiting = self.inbox.len();
                let inbox = self.inbox.clone();
                for msg in inbox.try_iter().take(waiting) {
                    self.handle(msg);
                }
                self.server.tick();
                self.next_tick += TICK;
                continue;
            }
            match self.inbox.recv_timeout(self.next_tick - now) {
                Ok(msg) => self.handle(msg),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => panic!("plugin host stopped"),
            }
        }
    }

    fn handle(&mut self, msg: Inbound<Link>) {
        match &msg {
            Inbound::PluginConnected { connection, .. } => self.plugin = Some(*connection),
            Inbound::PluginDisconnected { connection } if self.plugin == Some(*connection) => {
                self.plugin = None;
            }
            _ => {}
        }
        self.server.handle(msg);
        self.server.deliver();
    }
}

fn make_jwt(username: &str, roles: &[&str]) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let body = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&json!({
            "sub": format!("fk-{username}"),
            "preferred_username": username,
            "email": format!("{username}@ferrislabs.io"),
            "exp": 1_999_999_999u64,
            "realm_access": { "roles": roles }
        }))
        .unwrap(),
    );
    format!("{header}.{body}.sig")
}

#[derive(Default)]
struct MockBackend {
    expected_jwt: String,
    card_bearers: Vec<String>,
    cards: Vec<Value>,
    chat_bearers: Vec<String>,
    runs: Vec<Value>,
    run_poll_count: u32,
}

type SharedMock = Arc<Mutex<MockBackend>>;

fn bearer_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::to_owned)
}

fn build_mock_router(state: SharedMock) -> Router {
    Router::new()
        .route(
            "/realms/hypolia/protocol/openid-connect/auth/device",
            post(|| async {
                Json(json!({
                    "device_code": "dc-alice-1",
                    "user_code": "FERR-2026",
                    "verification_uri": "http://localhost:3333/device",
                    "verification_uri_complete": "http://localhost:3333/device?user_code=FERR-2026",
                    "expires_in": 300,
                    "interval": 1
                }))
            }),
        )
        .route(
            "/realms/hypolia/protocol/openid-connect/token",
            post(
                |State(mock): State<SharedMock>, Form(form): Form<HashMap<String, String>>| async move {
                    if form.get("grant_type").map(String::as_str)
                        == Some("urn:ietf:params:oauth:grant-type:device_code")
                    {
                        let jwt = mock.lock().unwrap().expected_jwt.clone();
                        Json(json!({
                            "access_token": jwt,
                            "refresh_token": "rt-alice",
                            "expires_in": 600
                        }))
                        .into_response()
                    } else {
                        StatusCode::BAD_REQUEST.into_response()
                    }
                },
            ),
        )
        .route(
            "/api/v1/team/agents",
            get(|State(mock): State<SharedMock>, headers: HeaderMap| async move {
                let expected = mock.lock().unwrap().expected_jwt.clone();
                if bearer_of(&headers).as_deref() != Some(expected.as_str()) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                Json(json!({
                    "items": [{
                        "id": "a1",
                        "slug": "forgeur",
                        "name": "Forgeur",
                        "role": "Développe les plugins mini-jeux",
                        "archived": false,
                        "paused": false
                    }]
                }))
                .into_response()
            }),
        )
        .route(
            "/api/v1/runs/active",
            get(|State(mock): State<SharedMock>, headers: HeaderMap| async move {
                let guard = mock.lock().unwrap();
                if bearer_of(&headers).as_deref() != Some(guard.expected_jwt.as_str()) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                Json(json!({ "items": guard.runs })).into_response()
            }),
        )
        .route(
            "/api/v1/cards",
            post(
                |State(mock): State<SharedMock>, headers: HeaderMap, Json(body): Json<Value>| async move {
                    let Some(bearer) = bearer_of(&headers) else {
                        return StatusCode::UNAUTHORIZED.into_response();
                    };
                    let mut guard = mock.lock().unwrap();
                    guard.card_bearers.push(bearer);
                    guard.cards.push(body);
                    (StatusCode::CREATED, Json(json!({"id": "card-1", "version": 1})))
                        .into_response()
                },
            ),
        )
        .route(
            "/api/v1/cards/{card}/agent-runs",
            post(|AxumPath(_card): AxumPath<String>, headers: HeaderMap| async move {
                if bearer_of(&headers).is_none() {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                (StatusCode::ACCEPTED, Json(json!({"run_id": "run-1"}))).into_response()
            }),
        )
        .route(
            "/api/v1/runs/{run_id}",
            get(
                |State(mock): State<SharedMock>,
                 AxumPath(run_id): AxumPath<String>,
                 headers: HeaderMap| async move {
                    if bearer_of(&headers).is_none() {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    let mut guard = mock.lock().unwrap();
                    guard.run_poll_count += 1;
                    let status = if guard.run_poll_count >= 2 {
                        "succeeded"
                    } else {
                        "running"
                    };
                    Json(json!({
                        "id": run_id,
                        "status": status,
                        "title": "Ajouter Mouton Glacé",
                        "card_id": "card-1",
                        "failure": null,
                        "agent": { "id": "a1", "name": "Forgeur" }
                    }))
                    .into_response()
                },
            ),
        )
        .route(
            "/api/v1/cards/{card_id}",
            get(
                |AxumPath(card_id): AxumPath<String>, headers: HeaderMap| async move {
                    if bearer_of(&headers).is_none() {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    Json(json!({
                        "card": { "id": card_id },
                        "comments": [{
                            "id": "cm-1",
                            "body": "## Mouton Glacé ajouté\n- Ralentissement 4s dans un rayon de 5 blocs.",
                            "author_subject": "forgeur",
                            "author_actor": "agent:forgeur",
                            "created_at": "2026-11-19T10:00:00Z"
                        }],
                        "runs": []
                    }))
                    .into_response()
                },
            ),
        )
        .route(
            "/api/v1/conversations",
            post(|State(mock): State<SharedMock>, headers: HeaderMap| async move {
                let Some(bearer) = bearer_of(&headers) else {
                    return StatusCode::UNAUTHORIZED.into_response();
                };
                mock.lock().unwrap().chat_bearers.push(bearer);
                (StatusCode::CREATED, Json(json!({"id": "conv-lobby"}))).into_response()
            }),
        )
        .route(
            "/api/v1/conversations/{id}/messages",
            post(|AxumPath(_id): AxumPath<String>, headers: HeaderMap| async move {
                if bearer_of(&headers).is_none() {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                let sse = "event: delta\ndata: {\"text\":\"Les moutons sont prêts !\"}\n\n\
                           event: done\ndata: {\"status\":\"complete\"}\n\n";
                ([("content-type", "text/event-stream")], sse).into_response()
            }),
        )
        .with_state(state)
}

fn interact_entity(server: &mut Server<Link>, player: &TestPlayer, entity_id: i32) {
    server.handle(Inbound::Input {
        player: player.id,
        input: PlayerInput::Interact {
            target: EntityId(entity_id),
            hand: Hand::Main,
            sneaking: false,
            location: Vec3::new(0.0, 1.0, 0.0),
        },
    });
    server.deliver();
}

fn send_input(server: &mut Server<Link>, player: &TestPlayer, input: PlayerInput) {
    server.handle(Inbound::Input {
        player: player.id,
        input,
    });
    server.deliver();
}

#[test]
fn lobby_spawns_pedestals_living_npcs_rich_chat_and_per_player_ferriskey_capitainerie() {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let host_listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let host_addr = host_listener.local_addr().unwrap();
    let (inbound_tx, inbox_rx) = crossbeam_channel::unbounded::<Inbound<Link>>();
    let (_stop_tx, stop_rx) = watch::channel(false);
    runtime.spawn(rspigot_plugins::serve(
        host_listener,
        None,
        inbound_tx,
        stop_rx,
    ));

    let alice_jwt = make_jwt("alice_fk", &["admin"]);
    let mock: SharedMock = Arc::new(Mutex::new(MockBackend {
        expected_jwt: alice_jwt.clone(),
        runs: vec![json!({
            "run": {
                "id": "r-1",
                "status": "running",
                "title": "Équilibrage SheepWars",
                "agent": {"id": "a1", "name": "Forgeur"}
            },
            "current_step": {"kind": "execute"},
            "steps_done": 2,
            "steps_total": 4
        })],
        ..MockBackend::default()
    }));

    let http_listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let http_addr = http_listener.local_addr().unwrap();
    let router = build_mock_router(Arc::clone(&mock));
    runtime.spawn(async move {
        let _ = axum::serve(http_listener, router).await;
    });

    let temp_dir = std::env::temp_dir().join(format!("fc_lobby_e2e_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let mut core = Core::new(inbox_rx);
    let _plugin = PluginProcess::start(host_addr, http_addr, &temp_dir);
    core.run_until("lobby plugin connects", |c| c.plugin.is_some());

    let mut alice = join(&mut core.server, "Alice");
    let mut bob = join(&mut core.server, "Bob");

    core.run_until("Alice receives welcome message and spawn teleport", |c| {
        confirm_teleports(&mut c.server, &mut alice);
        confirm_teleports(&mut c.server, &mut bob);
        alice.said("Hypolia")
    });

    let (mut sheepwars_npc, mut guard_npc, mut captain_npc) = (None, None, None);
    core.run_until(
        "Alice sees SheepWars pedestal, Guard, and Captain NPCs",
        |c| {
            confirm_teleports(&mut c.server, &mut alice);
            sheepwars_npc = alice.npc_entity_and_pos("SheepWars");
            guard_npc = alice.npc_entity_and_pos("Garde_Pont");
            captain_npc = alice.npc_entity_and_pos("Capitaine");
            sheepwars_npc.is_some() && guard_npc.is_some() && captain_npc.is_some()
        },
    );
    let (sheepwars_id, sheepwars_pos) = sheepwars_npc.unwrap();
    let (guard_id, guard_pos) = guard_npc.unwrap();
    let (captain_id, captain_pos) = captain_npc.unwrap();

    // 1. Walk to Guard & interact
    move_close_to(&mut core, &mut alice, guard_pos);
    interact_entity(&mut core.server, &alice, guard_id);
    core.run_until("Guard greets Alice", |_| {
        alice.said("Bienvenue dans la Citadelle Hypolia")
    });

    // 2. Walk to SheepWars pedestal NPC & click to join queue
    move_close_to(&mut core, &mut alice, sheepwars_pos);
    interact_entity(&mut core.server, &alice, sheepwars_id);
    core.run_until("Alice joins SheepWars queue", |_| {
        alice.said("file d'attente") && alice.label_has("En file : 1")
    });

    // 3. Rich Lobby Chat with @mention (replaces vanilla PlayerChat)
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::Chat("Salut @bob prêt pour SheepWars ?".to_owned()),
    );
    core.run_until("Bob receives formatted @mention chat", |_| {
        bob.said("@Bob") && !bob.vanilla_chat_seen()
    });

    // 4. Walk to Captain before FerrisKey login -> prompts Device Flow dialog
    move_close_to(&mut core, &mut alice, captain_pos);
    interact_entity(&mut core.server, &alice, captain_id);
    core.run_until("Alice receives FerrisKey login prompt dialog", |_| {
        alice
            .dialog_buttons()
            .iter()
            .any(|b| b == "hypolia:login_now")
    });

    // 5. Alice triggers FerrisKey Device Flow -> receives code -> auto-authenticates!
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::CustomClick {
            id: "hypolia:login_now".to_owned(),
            data: None,
        },
    );
    core.run_until(
        "Alice sees FerrisKey user_code prompt and completes auth",
        |_| alice.said("FERR-2026") && alice.said("alice_fk"),
    );

    // 6. Now Capitainerie discovers Forgeur agent & shows live progress bar
    let mut forgeur_npc = None;
    core.run_until("Forgeur agent spawns and displays active run", |_| {
        forgeur_npc = alice.npc_entity_and_pos("forgeur");
        forgeur_npc.is_some() && alice.label_has("Équilibrage SheepWars") && alice.label_has("2/4")
    });
    let (forgeur_id, forgeur_pos) = forgeur_npc.unwrap();

    // 7. Walk to Forgeur workstation & assign task using Alice's personal FerrisKey token
    move_close_to(&mut core, &mut alice, forgeur_pos);
    interact_entity(&mut core.server, &alice, forgeur_id);
    core.run_until("Alice gets Forgeur task dialog", |_| {
        alice
            .dialog_buttons()
            .iter()
            .any(|b| b == "hypolia:cap_run/forgeur")
    });
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::CustomClick {
            id: "hypolia:cap_run/forgeur".to_owned(),
            data: Some(
                r#"{"title":"Ajouter Mouton Glacé","description":"Ralentit la zone"}"#.to_owned(),
            ),
        },
    );
    core.run_until("Forgeur task starts under Alice's FerrisKey bearer", |_| {
        alice.said("Forgeur rejoint son établi")
    });
    {
        let guard = mock.lock().unwrap();
        assert_eq!(guard.card_bearers, vec![alice_jwt.clone()]);
        assert_eq!(guard.cards[0]["title"], "Ajouter Mouton Glacé");
    }

    // 7b. Run watcher detects completion -> BossBar + Title + RichMessage -> clicking opens full Dialog
    core.run_until("Alice receives Forgeur completion feedback", |_| {
        alice.event_has("Forgeur A TERMINÉ")
            && alice.event_has("Terminé :")
            && alice.said("Forgeur a terminé sa mission")
            && alice.said("Lire la réponse")
            && alice.said("Ouvrir sur Capitainerie")
    });
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::CustomClick {
            id: "hypolia:cap_result/run-1".to_owned(),
            data: None,
        },
    );
    core.run_until("Alice opens Forgeur full result dialog", |_| {
        alice.dialog_has("Mouton Glacé ajouté") && alice.dialog_has("Ralentissement 4s")
    });

    // 8. Walk back to Captain & chat over SSE using Alice's personal FerrisKey token
    move_close_to(&mut core, &mut alice, captain_pos);
    interact_entity(&mut core.server, &alice, captain_id);
    core.run_until("Alice opens private Captain channel", |_| {
        alice.said("Canal privé ouvert sous votre identité FerrisKey")
    });
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::Chat("Où en est l'arène ?".to_owned()),
    );
    core.run_until("Captain streams SSE reply", |_| {
        alice.said("Les moutons sont prêts !")
    });
    assert_eq!(mock.lock().unwrap().chat_bearers, vec![alice_jwt]);
    assert_eq!(core.server.game_rules().random_tick_speed, 0);
    assert_eq!(core.server.game_rules().fire_spread_radius, 0);

    // 9. Locked Options menu -> clicking Fly (12) and Speed (14) MUST work and update Abilities
    let opt_cursor = alice.log_cursor();
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::Command("options".to_owned()),
    );
    let mut opt_screen = None;
    core.run_until("Alice opens Options menu", |_| {
        opt_screen = alice.latest_screen_after(opt_cursor, "Paramètres");
        opt_screen.is_some()
    });
    let (opt_win, opt_state) = opt_screen.unwrap();

    let fly_cursor = alice.log_cursor();
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::ContainerClick(ContainerClick {
            container: opt_win,
            state_id: opt_state,
            slot: 12, // OPT_SLOT_FLIGHT
            button: 0,
            input: ClickInput::Pickup,
            changed: Vec::new(),
            carried: HashedSlot::EMPTY,
        }),
    );
    core.run_until("Clicking Fly in Options grants flight ability", |_| {
        alice.abilities_after(fly_cursor, |ab| ab.may_fly) && alice.said("Vol dans le Lobby activé")
    });

    let speed_cursor = alice.log_cursor();
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::ContainerClick(ContainerClick {
            container: opt_win,
            state_id: opt_state,
            slot: 14, // OPT_SLOT_SPEED
            button: 0,
            input: ClickInput::Pickup,
            changed: Vec::new(),
            carried: HashedSlot::EMPTY,
        }),
    );
    core.run_until(
        "Clicking Speed in Options increases walk & fly speed",
        |_| {
            alice.abilities_after(speed_cursor, |ab| {
                ab.may_fly && ab.walk_speed > 0.13 && ab.fly_speed > 0.06
            }) && alice.said("Vitesse réglée")
        },
    );
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::ContainerClose(opt_win),
    );

    // /fly toggles flight off and back on cleanly
    let cmd_fly_cursor = alice.log_cursor();
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::Command("fly".to_owned()),
    );
    core.run_until("/fly disables flight", |_| {
        alice.abilities_after(cmd_fly_cursor, |ab| !ab.may_fly)
    });

    // 10. /gadgets menu -> cycle Trail (21), equip FireworkRocket (11, free offline), fire it, then FeatherDash (10)
    let gad_cursor = alice.log_cursor();
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::Command("gadgets".to_owned()),
    );
    let mut gad_screen = None;
    core.run_until("Alice opens Gadgets menu", |_| {
        gad_screen = alice.latest_screen_after(gad_cursor, "Collectibles");
        gad_screen.is_some()
    });
    let (gad_win, gad_state) = gad_screen.unwrap();

    send_input(
        &mut core.server,
        &alice,
        PlayerInput::ContainerClick(ContainerClick {
            container: gad_win,
            state_id: gad_state,
            slot: 21, // GAD_SLOT_TRAIL
            button: 0,
            input: ClickInput::Pickup,
            changed: Vec::new(),
            carried: HashedSlot::EMPTY,
        }),
    );
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::ContainerClick(ContainerClick {
            container: gad_win,
            state_id: gad_state,
            slot: 11, // GAD_SLOT_ROCKET
            button: 0,
            input: ClickInput::Pickup,
            changed: Vec::new(),
            carried: HashedSlot::EMPTY,
        }),
    );
    core.run_until("Alice equips FireworkRocket from Gadgets menu", |_| {
        alice.said("Fusée d'Artifice")
    });
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::ContainerClose(gad_win),
    );

    // Activate FireworkRocket from slot 5 -> spawns a rocket entity & particles
    let sheep_cursor = alice.log_cursor();
    send_input(&mut core.server, &alice, PlayerInput::HeldSlot(5));
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::UseItem {
            hand: Hand::Main,
            sequence: 1,
            rotation: Rotation::default(),
        },
    );
    core.run_until("FireworkRocket spawns an entity and particles", |_| {
        alice.has_sheep_spawn_after(sheep_cursor) && alice.has_particle_after(sheep_cursor)
    });

    // Equip FeatherDash (slot 10) and fire it -> sets player velocity & particles
    let gad2_cursor = alice.log_cursor();
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::Command("gadgets".to_owned()),
    );
    let mut gad2_screen = None;
    core.run_until("Alice re-opens Gadgets menu", |_| {
        gad2_screen = alice.latest_screen_after(gad2_cursor, "Collectibles");
        gad2_screen.is_some()
    });
    let (gad2_win, gad2_state) = gad2_screen.unwrap();
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::ContainerClick(ContainerClick {
            container: gad2_win,
            state_id: gad2_state,
            slot: 10, // GAD_SLOT_FEATHER
            button: 0,
            input: ClickInput::Pickup,
            changed: Vec::new(),
            carried: HashedSlot::EMPTY,
        }),
    );
    core.run_until("Alice equips FeatherDash", |_| {
        alice.said("Propulseur Éthéré")
    });
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::ContainerClose(gad2_win),
    );

    let dash_cursor = alice.log_cursor();
    send_input(&mut core.server, &alice, PlayerInput::HeldSlot(5));
    send_input(
        &mut core.server,
        &alice,
        PlayerInput::UseItem {
            hand: Hand::Main,
            sequence: 2,
            rotation: Rotation::default(),
        },
    );
    core.run_until(
        "FeatherDash propels Alice with velocity & particles",
        |_| alice.has_velocity_after(dash_cursor) && alice.has_particle_after(dash_cursor),
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}
