use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use rspigot_plugin::{
    Error, ItemStack, Material, Npc, NpcPose, NpcSlot, NpcSpec, Particle, ParticleBurst, Player,
    Position, Rotation, Server, Sound, SoundCue, Uuid, Vector, World,
};

use crate::auth::AuthManager;
use crate::capitainerie::{
    ActiveRun, CapitainerieClient, CapitainerieError, ChatOutcome, RunStarted, TeamAgent,
    agent_label, captain_label, click_slug, is_working, sanitize_npc_name,
};
use crate::config::{
    AmbientNpcSpec, AmbientRole, LayoutStore, LobbyLayout, PedestalSpec, WaypointPose,
};
use crate::orchestrator::QueueCounts;
use hypolia_protocol::GameKind;

const AMBIENT_TICK: Duration = Duration::from_millis(1500);
const CAPITAINERIE_POLL: Duration = Duration::from_secs(3);

pub fn pedestal_game(spec: &PedestalSpec) -> Option<GameKind> {
    spec.active.then(|| GameKind::from_slug(&spec.game_id))?
}

pub fn pedestal_label(spec: &PedestalSpec, queued: QueueCounts) -> Option<String> {
    let game = pedestal_game(spec)?;
    let color = match game {
        GameKind::SheepWars => "§d",
        GameKind::TowerWars => "§b",
    };
    Some(format!(
        "{}\n§fEn jeu : §a{} §8• §fEn file : {color}{}\n{}",
        spec.title,
        queued.playing(game),
        queued.get(game),
        spec.subtitle
    ))
}

#[derive(Debug, Clone)]
pub enum NpcClickTarget {
    Pedestal(PedestalSpec),
    Ambient {
        npc: Npc,
        spec: AmbientNpcSpec,
        line: String,
    },
    Captain,
    Agent(TeamAgent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbientPhase {
    Walking {
        waypoint_idx: usize,
        ticks_walking: u8,
    },
    Acting {
        waypoint_idx: usize,
        ticks_left: u8,
    },
}

impl AmbientPhase {
    pub fn advance(self, still_walking: bool, waypoint_count: usize) -> (Self, AmbientStepAction) {
        if waypoint_count == 0 {
            return (self, AmbientStepAction::Idle);
        }
        match self {
            Self::Walking {
                waypoint_idx,
                ticks_walking,
            } => {
                if !still_walking || ticks_walking >= 8 {
                    (
                        Self::Acting {
                            waypoint_idx,
                            ticks_left: 2,
                        },
                        AmbientStepAction::PerformInteraction(waypoint_idx),
                    )
                } else {
                    (
                        Self::Walking {
                            waypoint_idx,
                            ticks_walking: ticks_walking.saturating_add(1),
                        },
                        AmbientStepAction::Idle,
                    )
                }
            }
            Self::Acting {
                waypoint_idx,
                ticks_left,
            } => {
                if ticks_left > 1 {
                    (
                        Self::Acting {
                            waypoint_idx,
                            ticks_left: ticks_left - 1,
                        },
                        AmbientStepAction::PerformInteraction(waypoint_idx),
                    )
                } else {
                    let next = (waypoint_idx + 1) % waypoint_count;
                    (
                        Self::Walking {
                            waypoint_idx: next,
                            ticks_walking: 0,
                        },
                        AmbientStepAction::WalkTo(next),
                    )
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbientStepAction {
    Idle,
    WalkTo(usize),
    PerformInteraction(usize),
}

#[derive(Debug, Clone)]
pub enum CaptainTurn {
    Busy,
    Ready(Option<String>),
}

#[derive(Debug, Clone)]
enum ChatState {
    Listening(Option<String>),
    Answering,
}

struct PedestalActor {
    npc: Npc,
    spec: PedestalSpec,
}

struct AmbientActor {
    npc: Npc,
    spec: AmbientNpcSpec,
    phase: AmbientPhase,
    dialogue_cursor: usize,
}

struct AgentActor {
    npc: Npc,
    agent: TeamAgent,
    slot_index: usize,
    working: bool,
    at_workstation: bool,
    label: String,
}

pub struct NpcDirector {
    layout: Arc<LayoutStore>,
    capitainerie: Option<Arc<CapitainerieClient>>,
    pedestals: RwLock<Vec<PedestalActor>>,
    ambients: RwLock<Vec<AmbientActor>>,
    captain: RwLock<Option<Npc>>,
    agents: RwLock<HashMap<String, AgentActor>>,
    active_runs_by_id: RwLock<HashMap<String, ActiveRun>>,
    chats: RwLock<HashMap<Uuid, ChatState>>,
    working_count: AtomicUsize,
    total_agents: AtomicUsize,
}

impl NpcDirector {
    pub fn new(layout: Arc<LayoutStore>, capitainerie_url: Option<String>) -> Self {
        Self {
            layout,
            capitainerie: capitainerie_url.map(|u| Arc::new(CapitainerieClient::new(u))),
            pedestals: RwLock::new(Vec::new()),
            ambients: RwLock::new(Vec::new()),
            captain: RwLock::new(None),
            agents: RwLock::new(HashMap::new()),
            active_runs_by_id: RwLock::new(HashMap::new()),
            chats: RwLock::new(HashMap::new()),
            working_count: AtomicUsize::new(0),
            total_agents: AtomicUsize::new(0),
        }
    }

    pub fn has_capitainerie(&self) -> bool {
        self.capitainerie.is_some()
    }

    pub fn capitainerie_client(&self) -> Option<Arc<CapitainerieClient>> {
        self.capitainerie.clone()
    }

    pub fn run_progress(&self, run_id: &str) -> Option<(Option<String>, u32, u32)> {
        let guard = self
            .active_runs_by_id
            .read()
            .unwrap_or_else(|e| e.into_inner());
        guard.get(run_id).map(|run| {
            (
                run.current_step.as_ref().map(|s| s.kind.clone()),
                run.steps_done,
                run.steps_total,
            )
        })
    }

    pub async fn agent_position(&self, agent_id: &str) -> Option<Position> {
        let npc = {
            let guard = self.agents.read().unwrap_or_else(|e| e.into_inner());
            guard.get(agent_id).map(|actor| actor.npc.clone())
        }?;
        match npc.get_state().await {
            Ok(state) => Some(state.npc.position),
            Err(_) => Some(npc.position),
        }
    }

    pub fn agent_counts(&self) -> (usize, usize) {
        (
            self.working_count.load(Ordering::Relaxed),
            self.total_agents.load(Ordering::Relaxed),
        )
    }

    pub async fn spawn_all(&self, server: &Server, queued: QueueCounts) {
        let snapshot = self.layout.snapshot();
        self.sync_pedestals(&server.world, &snapshot, queued).await;
        self.sync_ambients(&server.world, &snapshot).await;
        self.sync_captain(&server.world, &snapshot, false).await;
    }

    pub fn update_pedestal_labels(&self, queued: QueueCounts) {
        let guard = self.pedestals.read().unwrap_or_else(|e| e.into_inner());
        for actor in guard.iter() {
            if let Some(label) = pedestal_label(&actor.spec, queued) {
                actor.npc.set_label(label);
            }
        }
    }

    pub fn lookup_entity(&self, entity_id: i32) -> Option<NpcClickTarget> {
        {
            let pedestals = self.pedestals.read().unwrap_or_else(|e| e.into_inner());
            if let Some(found) = pedestals.iter().find(|a| a.npc.entity_id == entity_id) {
                return Some(NpcClickTarget::Pedestal(found.spec.clone()));
            }
        }

        {
            let mut ambients = self.ambients.write().unwrap_or_else(|e| e.into_inner());
            if let Some(found) = ambients.iter_mut().find(|a| a.npc.entity_id == entity_id) {
                let line = if found.spec.dialogues.is_empty() {
                    "Bienvenue sur Hypolia !".to_owned()
                } else {
                    let idx = found.dialogue_cursor % found.spec.dialogues.len();
                    found.dialogue_cursor = found.dialogue_cursor.wrapping_add(1);
                    found.spec.dialogues[idx].clone()
                };
                return Some(NpcClickTarget::Ambient {
                    npc: found.npc.clone(),
                    spec: found.spec.clone(),
                    line,
                });
            }
        }

        {
            let captain = self.captain.read().unwrap_or_else(|e| e.into_inner());
            if captain
                .as_ref()
                .is_some_and(|npc| npc.entity_id == entity_id)
            {
                return Some(NpcClickTarget::Captain);
            }
        }

        let agents = self.agents.read().unwrap_or_else(|e| e.into_inner());
        agents
            .values()
            .find(|a| a.npc.entity_id == entity_id)
            .map(|a| NpcClickTarget::Agent(a.agent.clone()))
    }

    pub fn agent_by_click_slug(&self, slug: &str) -> Option<TeamAgent> {
        let agents = self.agents.read().unwrap_or_else(|e| e.into_inner());
        agents
            .values()
            .find(|a| click_slug(&a.agent.slug) == slug)
            .map(|a| a.agent.clone())
    }

    pub async fn start_agent_task(
        &self,
        bearer: &str,
        agent: &TeamAgent,
        title: &str,
        description: &str,
    ) -> Result<RunStarted, CapitainerieError> {
        let Some(client) = &self.capitainerie else {
            return Err(CapitainerieError::Chat {
                code: "not_connected".to_owned(),
                message: "CAPITAINERIE_URL n'est pas configurée".to_owned(),
            });
        };
        client
            .start_task(bearer, &agent.id, title, description)
            .await
    }

    pub fn toggle_captain_chat(&self, player_uuid: Uuid) -> bool {
        let mut guard = self.chats.write().unwrap_or_else(|e| e.into_inner());
        if guard.remove(&player_uuid).is_some() {
            false
        } else {
            guard.insert(player_uuid, ChatState::Listening(None));
            true
        }
    }

    pub fn is_in_captain_chat(&self, player_uuid: Uuid) -> bool {
        self.chats
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&player_uuid)
    }

    pub fn leave_captain_chat(&self, player_uuid: Uuid) {
        self.chats
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player_uuid);
    }

    pub fn is_quit_word(text: &str) -> bool {
        matches!(
            text.trim().to_ascii_lowercase().as_str(),
            "quitter" | "quit" | "stop" | "exit"
        )
    }

    pub fn begin_captain_turn(&self, player_uuid: Uuid) -> CaptainTurn {
        let mut guard = self.chats.write().unwrap_or_else(|e| e.into_inner());
        match guard.get(&player_uuid).cloned() {
            Some(ChatState::Listening(conv)) => {
                guard.insert(player_uuid, ChatState::Answering);
                CaptainTurn::Ready(conv)
            }
            _ => CaptainTurn::Busy,
        }
    }

    pub fn end_captain_turn(&self, player_uuid: Uuid, conversation: Option<String>) {
        let mut guard = self.chats.write().unwrap_or_else(|e| e.into_inner());
        if guard.contains_key(&player_uuid) {
            guard.insert(player_uuid, ChatState::Listening(conversation));
        }
    }

    pub async fn chat_with_captain(
        &self,
        bearer: &str,
        player: &Player,
        conversation: Option<&str>,
        message: &str,
    ) -> Result<ChatOutcome, CapitainerieError> {
        let Some(client) = &self.capitainerie else {
            return Err(CapitainerieError::Chat {
                code: "captain_not_configured".to_owned(),
                message: "CAPITAINERIE_URL non configurée".to_owned(),
            });
        };
        client
            .chat(bearer, conversation, message, |line| {
                if !line.is_empty() {
                    player.message.send(format!("§6Capitaine §8» §f{line}"));
                }
            })
            .await
    }

    pub async fn run_background_loops(self: Arc<Self>, server: Server, auth: Arc<AuthManager>) {
        let dir_ambient = Arc::clone(&self);
        let srv_ambient = server.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(AMBIENT_TICK);
            loop {
                ticker.tick().await;
                dir_ambient.tick_ambients(&srv_ambient.world).await;
            }
        });

        let mut cap_ticker = tokio::time::interval(CAPITAINERIE_POLL);
        loop {
            cap_ticker.tick().await;
            self.tick_capitainerie(&server.world, &auth).await;
        }
    }

    async fn tick_ambients(&self, world: &World) {
        let snapshot_actors: Vec<(usize, Npc, AmbientNpcSpec, AmbientPhase)> = {
            let guard = self.ambients.read().unwrap_or_else(|e| e.into_inner());
            guard
                .iter()
                .enumerate()
                .map(|(i, a)| (i, a.npc.clone(), a.spec.clone(), a.phase))
                .collect()
        };

        for (idx, npc, spec, phase) in snapshot_actors {
            let still_walking = match npc.get_state().await {
                Ok(state) => state.walking,
                Err(_) => false,
            };
            let (next_phase, action) = phase.advance(still_walking, spec.waypoints.len());
            match action {
                AmbientStepAction::Idle => {}
                AmbientStepAction::WalkTo(wp_idx) => {
                    npc.set_pose(NpcPose::Standing);
                    npc.stop_looking();
                    if let Some(target) = spec.waypoints.get(wp_idx) {
                        npc.walk_to_at(target.position(), spec.walk_speed);
                    }
                }
                AmbientStepAction::PerformInteraction(wp_idx) => {
                    let pos = spec
                        .waypoints
                        .get(wp_idx)
                        .map_or(npc.position, |wp| wp.position());
                    perform_ambient_world_interaction(world, &npc, spec.role, pos);
                }
            }

            let mut guard = self.ambients.write().unwrap_or_else(|e| e.into_inner());
            if let Some(slot) = guard.get_mut(idx) {
                slot.phase = next_phase;
            }
        }
    }

    async fn tick_capitainerie(&self, world: &World, auth: &AuthManager) {
        let layout = self.layout.snapshot();
        let Some(client) = &self.capitainerie else {
            // Keep portal ambient particles alive even without Capitainerie backend
            spawn_portal_aura(world, &layout);
            return;
        };
        spawn_portal_aura(world, &layout);

        let Some(bearer) = auth.observer_token().await else {
            self.update_captain_status(false, true);
            return;
        };

        let (agents_res, runs_res) =
            tokio::join!(client.agents(&bearer), client.active_runs(&bearer));
        let (Ok(agents), Ok(runs)) = (agents_res, runs_res) else {
            self.update_captain_status(false, true);
            return;
        };

        self.update_captain_status(true, true);
        self.reconcile_agents(world, &layout, agents, &runs).await;
    }

    fn update_captain_status(&self, online: bool, configured: bool) {
        if let Some(npc) = self
            .captain
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            npc.set_label(captain_label(online, configured));
        }
    }

    async fn reconcile_agents(
        &self,
        world: &World,
        layout: &LobbyLayout,
        agents: Vec<TeamAgent>,
        runs: &[ActiveRun],
    ) {
        {
            let mut by_id = self
                .active_runs_by_id
                .write()
                .unwrap_or_else(|e| e.into_inner());
            by_id.clear();
            for run in runs {
                by_id.insert(run.run.id.clone(), run.clone());
            }
        }

        let mut run_by_agent: HashMap<&str, &ActiveRun> = HashMap::new();
        for run in runs {
            if let Some(agent_ref) = &run.run.agent {
                let replace = run_by_agent
                    .get(agent_ref.id.as_str())
                    .is_none_or(|prev| prev.run.status != "running" && run.run.status == "running");
                if replace {
                    run_by_agent.insert(agent_ref.id.as_str(), run);
                }
            }
        }

        let visible: Vec<TeamAgent> = agents.into_iter().filter(|a| !a.archived).collect();
        let keep_ids: HashSet<&str> = visible.iter().map(|a| a.id.as_str()).collect();

        // Remove archived/deleted agents
        {
            let mut guard = self.agents.write().unwrap_or_else(|e| e.into_inner());
            guard.retain(|id, actor| {
                let keep = keep_ids.contains(id.as_str());
                if !keep {
                    actor.npc.remove();
                }
                keep
            });
        }

        let mut taken_names: HashSet<String> = {
            let guard = self.agents.read().unwrap_or_else(|e| e.into_inner());
            guard.values().map(|a| a.npc.name.clone()).collect()
        };
        taken_names.insert("Capitaine".to_owned());

        let mut working_now = 0usize;
        let total_now = visible.len();

        for (slot_idx, agent) in visible.into_iter().enumerate() {
            let run = run_by_agent.get(agent.id.as_str()).copied();
            let working = is_working(run);
            if working {
                working_now += 1;
            }
            let label = agent_label(&agent, run);
            let idle_pose = agent_idle_pose(layout, slot_idx);
            let work_pose = agent_workstation_pose(layout, slot_idx);

            let existing_npc = {
                let guard = self.agents.read().unwrap_or_else(|e| e.into_inner());
                guard.get(&agent.id).map(|a| {
                    (
                        a.npc.clone(),
                        a.working,
                        a.at_workstation,
                        a.label.clone(),
                        a.slot_index,
                    )
                })
            };

            match existing_npc {
                Some((npc, was_working, mut at_station, prev_label, prev_slot)) => {
                    if prev_label != label {
                        npc.set_label(label.clone());
                    }
                    if working && !was_working {
                        at_station = false;
                        npc.set_pose(NpcPose::Standing);
                        npc.stop_looking();
                        npc.walk_to_at(work_pose.position(), 3.2);
                    } else if !working && (was_working || prev_slot != slot_idx) {
                        at_station = false;
                        npc.set_pose(NpcPose::Standing);
                        npc.walk_to_at(idle_pose.position(), 2.8);
                    } else if working {
                        let walking = npc.get_state().await.is_ok_and(|s| s.walking);
                        if !walking {
                            if !at_station {
                                npc.set_rotation(work_pose.rotation());
                                npc.set_pose(NpcPose::Crouching);
                                at_station = true;
                            }
                            npc.swing_main_hand();
                            world.particle.spawn(
                                ParticleBurst::new(
                                    Particle::Enchant,
                                    Position {
                                        x: work_pose.x,
                                        y: work_pose.y + 1.2,
                                        z: work_pose.z,
                                    },
                                )
                                .count(6)
                                .spread(Vector {
                                    x: 0.25,
                                    y: 0.25,
                                    z: 0.25,
                                })
                                .speed(0.15),
                            );
                        }
                    } else {
                        let walking = npc.get_state().await.is_ok_and(|s| s.walking);
                        if !walking {
                            npc.look_at_nearest(9.0);
                        }
                    }

                    let mut guard = self.agents.write().unwrap_or_else(|e| e.into_inner());
                    if let Some(actor) = guard.get_mut(&agent.id) {
                        actor.agent = agent;
                        actor.slot_index = slot_idx;
                        actor.working = working;
                        actor.at_workstation = at_station;
                        actor.label = label;
                    }
                }
                None => {
                    let safe_name = sanitize_npc_name(&agent.slug, &taken_names);
                    taken_names.insert(safe_name.clone());
                    let start_pose = if working { work_pose } else { idle_pose };
                    let key = format!("cap:agent:{}", agent.id);
                    if let Ok(npc) = adopt_or_create(
                        world,
                        &key,
                        &safe_name,
                        start_pose.position(),
                        start_pose.rotation(),
                    )
                    .await
                    {
                        npc.set_equipment(
                            NpcSlot::MainHand,
                            Some(ItemStack::new(Material::WritableBook, 1)),
                        );
                        npc.set_equipment(
                            NpcSlot::OffHand,
                            Some(ItemStack::new(Material::Lantern, 1)),
                        );
                        npc.set_pose(if working {
                            NpcPose::Crouching
                        } else {
                            NpcPose::Standing
                        });
                        npc.set_label(label.clone());
                        if !working {
                            npc.look_at_nearest(9.0);
                        }
                        self.agents
                            .write()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(
                                agent.id.clone(),
                                AgentActor {
                                    npc,
                                    agent,
                                    slot_index: slot_idx,
                                    working,
                                    at_workstation: working,
                                    label,
                                },
                            );
                    }
                }
            }
        }

        self.working_count.store(working_now, Ordering::Relaxed);
        self.total_agents.store(total_now, Ordering::Relaxed);
    }

    async fn sync_pedestals(&self, world: &World, layout: &LobbyLayout, queued: QueueCounts) {
        let mut actors = Vec::with_capacity(layout.pedestals.len());
        for spec in &layout.pedestals {
            let key = format!("pedestal:{}", spec.id);
            match adopt_or_create(
                world,
                &key,
                &spec.npc_name,
                spec.pose.position(),
                spec.pose.rotation(),
            )
            .await
            {
                Ok(npc) => {
                    npc.set_pose(NpcPose::Standing);
                    npc.set_equipment(
                        NpcSlot::MainHand,
                        spec.main_material().map(|m| ItemStack::new(m, 1)),
                    );
                    npc.set_equipment(
                        NpcSlot::OffHand,
                        spec.off_material().map(|m| ItemStack::new(m, 1)),
                    );
                    let label = pedestal_label(spec, queued)
                        .unwrap_or_else(|| format!("{}\n{}", spec.title, spec.subtitle));
                    npc.set_label(label);
                    npc.look_at_nearest(10.0);
                    actors.push(PedestalActor {
                        npc,
                        spec: spec.clone(),
                    });
                }
                Err(err) => {
                    tracing::warn!("Impossible de placer le PNJ de plot {}: {err}", spec.id);
                }
            }
        }
        *self.pedestals.write().unwrap_or_else(|e| e.into_inner()) = actors;
    }

    async fn sync_ambients(&self, world: &World, layout: &LobbyLayout) {
        let mut actors = Vec::with_capacity(layout.ambient_npcs.len());
        for spec in &layout.ambient_npcs {
            let start = spec.waypoints.first().copied().unwrap_or(layout.spawn);
            let key = format!("ambient:{}", spec.id);
            match adopt_or_create(
                world,
                &key,
                &spec.npc_name,
                start.position(),
                start.rotation(),
            )
            .await
            {
                Ok(npc) => {
                    equip_ambient_role(&npc, spec.role);
                    npc.set_label(format!("{}\n{}", spec.display_name, spec.subtitle));
                    npc.set_pose(NpcPose::Standing);
                    actors.push(AmbientActor {
                        npc,
                        spec: spec.clone(),
                        phase: AmbientPhase::Acting {
                            waypoint_idx: 0,
                            ticks_left: 1,
                        },
                        dialogue_cursor: 0,
                    });
                }
                Err(err) => {
                    tracing::warn!("Impossible de placer le PNJ d'ambiance {}: {err}", spec.id);
                }
            }
        }
        *self.ambients.write().unwrap_or_else(|e| e.into_inner()) = actors;
    }

    async fn sync_captain(&self, world: &World, layout: &LobbyLayout, online: bool) {
        match adopt_or_create(
            world,
            "cap:captain",
            "Capitaine",
            layout.captain.position(),
            layout.captain.rotation(),
        )
        .await
        {
            Ok(npc) => {
                npc.set_pose(NpcPose::Standing);
                npc.set_equipment(
                    NpcSlot::MainHand,
                    Some(ItemStack::new(Material::Spyglass, 1)),
                );
                npc.set_label(captain_label(online, self.has_capitainerie()));
                npc.look_at_nearest(10.0);
                *self.captain.write().unwrap_or_else(|e| e.into_inner()) = Some(npc);
            }
            Err(err) => tracing::warn!("Impossible de placer le Capitaine : {err}"),
        }
    }
}

fn agent_idle_pose(layout: &LobbyLayout, index: usize) -> WaypointPose {
    let offset_z = (index as f64) * 2.2;
    let offset_x = (index as f64) * layout.agent_idle_step_x;
    WaypointPose::new(
        layout.agent_idle_origin.x + offset_x,
        layout.agent_idle_origin.y,
        layout.agent_idle_origin.z + offset_z,
        layout.agent_idle_origin.yaw,
        layout.agent_idle_origin.pitch,
    )
}

fn agent_workstation_pose(layout: &LobbyLayout, index: usize) -> WaypointPose {
    if layout.agent_workstations.is_empty() {
        return agent_idle_pose(layout, index);
    }
    layout.agent_workstations[index % layout.agent_workstations.len()]
}

fn equip_ambient_role(npc: &Npc, role: AmbientRole) {
    match role {
        AmbientRole::Guard => {
            npc.set_equipment(
                NpcSlot::MainHand,
                Some(ItemStack::new(Material::IronSword, 1)),
            );
            npc.set_equipment(NpcSlot::OffHand, Some(ItemStack::new(Material::Shield, 1)));
        }
        AmbientRole::Lumberjack => {
            npc.set_equipment(
                NpcSlot::MainHand,
                Some(ItemStack::new(Material::IronAxe, 1)),
            );
            npc.set_equipment(
                NpcSlot::OffHand,
                Some(ItemStack::new(Material::SpruceLog, 1)),
            );
        }
        AmbientRole::PortalMage => {
            npc.set_equipment(
                NpcSlot::MainHand,
                Some(ItemStack::new(Material::AmethystShard, 1)),
            );
            npc.set_equipment(
                NpcSlot::OffHand,
                Some(ItemStack::new(Material::EnderPearl, 1)),
            );
        }
        AmbientRole::Innkeeper => {
            npc.set_equipment(
                NpcSlot::MainHand,
                Some(ItemStack::new(Material::Lantern, 1)),
            );
            npc.set_equipment(NpcSlot::OffHand, Some(ItemStack::new(Material::Book, 1)));
        }
    }
}

fn perform_ambient_world_interaction(world: &World, npc: &Npc, role: AmbientRole, pos: Position) {
    let head = Position {
        x: pos.x,
        y: pos.y + 1.3,
        z: pos.z,
    };
    match role {
        AmbientRole::Guard => {
            npc.set_pose(NpcPose::Standing);
            npc.look_at_nearest(12.0);
            npc.swing_main_hand();
        }
        AmbientRole::Lumberjack => {
            npc.set_pose(NpcPose::Crouching);
            npc.swing_main_hand();
            world.sound.play(
                SoundCue::new(Sound::BlockWoodHit)
                    .volume(0.55)
                    .pitch(0.95)
                    .at(pos),
            );
            world.particle.spawn(
                ParticleBurst::new(Particle::Crit, head)
                    .count(5)
                    .spread(Vector {
                        x: 0.2,
                        y: 0.2,
                        z: 0.2,
                    })
                    .speed(0.1),
            );
        }
        AmbientRole::PortalMage => {
            npc.set_pose(NpcPose::Standing);
            npc.swing_main_hand();
            world.particle.spawn(
                ParticleBurst::new(Particle::Portal, head)
                    .count(12)
                    .spread(Vector {
                        x: 0.4,
                        y: 0.5,
                        z: 0.4,
                    })
                    .speed(0.35),
            );
            world.sound.play(
                SoundCue::new(Sound::BlockNoteBlockChime)
                    .volume(0.4)
                    .pitch(1.4)
                    .at(pos),
            );
        }
        AmbientRole::Innkeeper => {
            npc.set_pose(NpcPose::Standing);
            npc.look_at_nearest(10.0);
            world.particle.spawn(
                ParticleBurst::new(Particle::HappyVillager, head)
                    .count(3)
                    .spread(Vector {
                        x: 0.3,
                        y: 0.3,
                        z: 0.3,
                    })
                    .speed(0.02),
            );
        }
    }
}

fn spawn_portal_aura(world: &World, layout: &LobbyLayout) {
    if let Some(portal) = layout.pois.get("portal") {
        let plane = portal.project_forward(2.5);
        world.particle.spawn(
            ParticleBurst::new(
                Particle::Portal,
                Position {
                    x: plane.x,
                    y: plane.y + 3.0,
                    z: plane.z,
                },
            )
            .count(20)
            .spread(Vector {
                x: 2.5,
                y: 2.5,
                z: 0.5,
            })
            .speed(0.4),
        );
    }
}

async fn adopt_or_create(
    world: &World,
    key: &str,
    name: &str,
    position: Position,
    rotation: Rotation,
) -> Result<Npc, Error> {
    if let Some(existing) = world.npcs.get(key).await? {
        existing.stop_walking();
        existing.teleport(position);
        existing.set_rotation(rotation);
        return Ok(existing);
    }
    world
        .npcs
        .create(NpcSpec::new(key, name, position).rotation(rotation))
        .await
}

#[cfg(test)]
mod tests {
    #[test]
    fn pedestal_labels_show_the_queue_of_their_own_game() {
        let layout = LobbyLayout::default();
        let counts = QueueCounts {
            sheepwars: 2,
            towerwars: 7,
            sheepwars_playing: 6,
            towerwars_playing: 3,
        };
        let label_of = |id: &str| {
            let spec = layout.pedestals.iter().find(|p| p.id == id).expect("spec");
            pedestal_label(spec, counts)
        };
        assert!(label_of("sheepwars").expect("label").contains("§d2"));
        assert!(label_of("towerwars").expect("label").contains("§b7"));
        assert!(
            label_of("sheepwars")
                .expect("label")
                .contains("En jeu : §a6")
        );
        assert!(
            label_of("towerwars")
                .expect("label")
                .contains("En jeu : §a3")
        );
        assert_eq!(label_of("rush"), None);
    }

    use super::*;

    #[test]
    fn ambient_phase_walks_acts_and_cycles_waypoints() {
        let phase = AmbientPhase::Walking {
            waypoint_idx: 0,
            ticks_walking: 1,
        };
        // Still walking -> stays Walking
        let (p1, a1) = phase.advance(true, 3);
        assert_eq!(a1, AmbientStepAction::Idle);
        assert_eq!(
            p1,
            AmbientPhase::Walking {
                waypoint_idx: 0,
                ticks_walking: 2
            }
        );

        // Arrived (!walking) -> transitions to Acting(ticks_left: 2)
        let (p2, a2) = p1.advance(false, 3);
        assert_eq!(a2, AmbientStepAction::PerformInteraction(0));
        assert_eq!(
            p2,
            AmbientPhase::Acting {
                waypoint_idx: 0,
                ticks_left: 2
            }
        );

        // Second interaction tick
        let (p3, a3) = p2.advance(false, 3);
        assert_eq!(a3, AmbientStepAction::PerformInteraction(0));
        assert_eq!(
            p3,
            AmbientPhase::Acting {
                waypoint_idx: 0,
                ticks_left: 1
            }
        );

        // Next tick -> walks to waypoint 1
        let (p4, a4) = p3.advance(false, 3);
        assert_eq!(a4, AmbientStepAction::WalkTo(1));
        assert_eq!(
            p4,
            AmbientPhase::Walking {
                waypoint_idx: 1,
                ticks_walking: 0
            }
        );
    }
}
