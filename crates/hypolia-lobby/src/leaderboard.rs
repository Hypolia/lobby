use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use hypolia_protocol::{GameKind, Leaderboard, LeaderboardEntry};
use rspigot_plugin::{Billboard, DisplayProps, Entity, InteractionBox, Server, TextDisplay, World};
use tokio::sync::Mutex;
use tokio::time::{Instant, MissedTickBehavior, interval_at};

use crate::config::LayoutStore;
use crate::orchestrator::OrchestratorClient;
use crate::progress_menus::game_label;

pub const BOARD_SIZE: u32 = 10;
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const META_KEY: &str = "hypolia:leaderboard";
const HITBOX_KEY: &str = "hypolia:leaderboard-hitbox";
const FOOTER: &str = "§8Clic droit : changer de jeu";
/// Title, game, one line per rank and the footer: the board is always this tall.
pub const BOARD_LINES: usize = BOARD_SIZE as usize + 3;
const LINE_HEIGHT: f32 = 0.25;
const HITBOX_WIDTH: f32 = 3.5;
const HITBOX_HEIGHT: f32 = BOARD_LINES as f32 * LINE_HEIGHT + 0.1;

const fn game_color(game: GameKind) -> &'static str {
    match game {
        GameKind::SheepWars => "§d",
        GameKind::TowerWars => "§b",
        GameKind::DeACoudre => "§6",
    }
}

fn plain_name(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '§' && !c.is_control())
        .collect()
}

pub fn format_entry(entry: &LeaderboardEntry) -> String {
    format!(
        "§e{}. §f{} §7— §a{}",
        entry.rank,
        plain_name(&entry.name),
        entry.wins
    )
}

pub fn format_board(game: GameKind, board: Option<&Leaderboard>) -> String {
    let mut lines = vec![
        "§6§lMEILLEURS JOUEURS".to_owned(),
        format!("{}{} §7· victoires", game_color(game), game_label(game)),
    ];
    match board {
        None => lines.push("§7Classement indisponible".to_owned()),
        Some(board) if board.entries.is_empty() => {
            lines.push("§7Aucune partie gagnée pour l'instant".to_owned());
        }
        Some(board) => lines.extend(
            board
                .entries
                .iter()
                .take(BOARD_SIZE as usize)
                .map(format_entry),
        ),
    }
    lines.resize(BOARD_LINES - 1, " ".to_owned());
    lines.push(FOOTER.to_owned());
    lines.join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconcile {
    pub remove: Vec<i32>,
    pub spawn: bool,
}

/// Decides what to do with the leaderboard displays found in the world so that
/// at most one exists, and none when no position is configured.
pub fn reconcile(configured: bool, tracked: Option<i32>, found: &[i32]) -> Reconcile {
    let keep = tracked.filter(|id| configured && found.contains(id));
    Reconcile {
        remove: found
            .iter()
            .copied()
            .filter(|id| Some(*id) != keep)
            .collect(),
        spawn: configured && keep.is_none(),
    }
}

/// Chunk holding a block position. The server only summons entities in loaded
/// chunks, and the lobby is often empty, so the board's chunk is force loaded.
pub fn chunk_of(x: f64, z: f64) -> (i32, i32) {
    ((x / 16.0).floor() as i32, (z / 16.0).floor() as i32)
}

#[derive(Debug, Default)]
struct Slot {
    display: Option<i32>,
    hitbox: Option<i32>,
    chunk: Option<(i32, i32)>,
}

struct Scan {
    plan: Reconcile,
    found: Vec<Entity>,
}

impl Scan {
    fn tracked(&self, id: Option<i32>) -> Option<&Entity> {
        self.found.iter().find(|e| Some(e.entity_id) == id)
    }
}

/// Lists the entities carrying `key`, and removes the ones that must not stay.
async fn scan(world: &World, key: &str, configured: bool, tracked: Option<i32>) -> Option<Scan> {
    let found = match world.entities.with_meta(key).await {
        Ok(found) => found,
        Err(err) => {
            tracing::warn!("Impossible de lister les entités {key} : {err}");
            return None;
        }
    };
    let ids: Vec<i32> = found.iter().map(|e| e.entity_id).collect();
    let plan = reconcile(configured, tracked, &ids);
    for entity in found.iter().filter(|e| plan.remove.contains(&e.entity_id)) {
        entity.remove();
    }
    Some(Scan { plan, found })
}

async fn spawn_marked(
    world: &World,
    key: &str,
    position: rspigot_plugin::Position,
    spec: impl Into<rspigot_plugin::DisplaySpec>,
) -> Option<i32> {
    match world.objects.spawn_display(position, spec).await {
        Ok(entity) => {
            entity.meta.set(key, true);
            Some(entity.entity_id)
        }
        Err(err) => {
            tracing::warn!("Impossible de placer {key} : {err}");
            None
        }
    }
}

pub struct LeaderboardHud {
    layout: Arc<LayoutStore>,
    orchestrator: Option<Arc<OrchestratorClient>>,
    cache: RwLock<BTreeMap<GameKind, Leaderboard>>,
    shown: AtomicUsize,
    slot: Mutex<Slot>,
}

impl LeaderboardHud {
    pub fn new(layout: Arc<LayoutStore>, orchestrator: Option<Arc<OrchestratorClient>>) -> Self {
        Self {
            layout,
            orchestrator,
            cache: RwLock::new(BTreeMap::new()),
            shown: AtomicUsize::new(0),
            slot: Mutex::new(Slot::default()),
        }
    }

    pub fn shown_game(&self) -> GameKind {
        GameKind::ALL[self.shown.load(Ordering::Relaxed) % GameKind::ALL.len()]
    }

    fn advance(&self) {
        self.shown.fetch_add(1, Ordering::Relaxed);
    }

    pub async fn is_hitbox(&self, entity_id: i32) -> bool {
        self.slot.lock().await.hitbox == Some(entity_id)
    }

    /// Shows the next game on the board, for every player.
    pub async fn switch_game(&self, world: &World) -> GameKind {
        self.advance();
        self.redraw(world).await;
        self.shown_game()
    }

    pub fn text(&self) -> String {
        let game = self.shown_game();
        let cache = self.cache.read().unwrap_or_else(|e| e.into_inner());
        format_board(game, cache.get(&game))
    }

    fn has_data(&self) -> bool {
        !self
            .cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    fn spec(&self) -> TextDisplay {
        TextDisplay::new(self.text()).props(DisplayProps::default().billboard(Billboard::Center))
    }

    /// Fetches every game; a failure keeps the previous data of that game.
    pub async fn refresh(&self) {
        let Some(orchestrator) = &self.orchestrator else {
            return;
        };
        for game in GameKind::ALL {
            match orchestrator.fetch_leaderboard(game, BOARD_SIZE).await {
                Ok(board) => {
                    self.cache
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(game, board);
                }
                Err(err) => {
                    tracing::warn!("Classement {} indisponible : {err}", game.slug());
                }
            }
        }
    }

    /// Brings the world in line with the layout: spawns, moves or removes the
    /// single leaderboard display. Safe to call as often as needed.
    pub async fn sync(&self, world: &World) {
        let spot = self.layout.snapshot().leaderboard;
        if spot.is_some() && !self.has_data() {
            self.refresh().await;
        }
        let mut slot = self.slot.lock().await;
        let wanted_chunk = spot.map(|s| chunk_of(s.x, s.z));
        if slot.chunk != wanted_chunk {
            if let Some((x, z)) = slot.chunk {
                world.chunks.at(x, z).set_force_loaded(false);
            }
            if let Some((x, z)) = wanted_chunk {
                world.chunks.at(x, z).set_force_loaded(true);
            }
            slot.chunk = wanted_chunk;
        }
        if let Some((x, z)) = wanted_chunk
            && let Err(err) = world.chunks.at(x, z).get_state(true).await
        {
            tracing::warn!("Impossible de charger le chunk du tableau : {err}");
            return;
        }
        let Some(board) = scan(world, META_KEY, spot.is_some(), slot.display).await else {
            return;
        };
        let Some(hitbox) = scan(world, HITBOX_KEY, spot.is_some(), slot.hitbox).await else {
            return;
        };
        let Some(spot) = spot else {
            slot.display = None;
            slot.hitbox = None;
            return;
        };
        let position = spot.position();
        if board.plan.spawn {
            slot.display = spawn_marked(world, META_KEY, position, self.spec()).await;
        } else if let Some(entity) = board.tracked(slot.display) {
            entity.teleport(position);
            entity.display.edit(self.spec());
        }
        let area = || InteractionBox::new(HITBOX_WIDTH, HITBOX_HEIGHT).response(true);
        if hitbox.plan.spawn {
            slot.hitbox = spawn_marked(world, HITBOX_KEY, position, area()).await;
        } else if let Some(entity) = hitbox.tracked(slot.hitbox) {
            entity.teleport(position);
        }
    }

    async fn redraw(&self, world: &World) {
        let slot = self.slot.lock().await;
        let Some(id) = slot.display else {
            return;
        };
        match world.entities.with_meta(META_KEY).await {
            Ok(found) => {
                if let Some(entity) = found.iter().find(|e| e.entity_id == id) {
                    entity.display.edit(self.spec());
                }
            }
            Err(err) => tracing::warn!("Impossible de rafraîchir le tableau : {err}"),
        }
    }

    pub async fn run(self: Arc<Self>, server: Server) {
        self.sync(&server.world).await;
        let mut refresh = interval_at(Instant::now() + REFRESH_INTERVAL, REFRESH_INTERVAL);
        refresh.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            refresh.tick().await;
            if self.layout.snapshot().leaderboard.is_some() {
                self.refresh().await;
                self.sync(&server.world).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rank: u32, name: &str, wins: u32) -> LeaderboardEntry {
        LeaderboardEntry {
            rank,
            name: name.to_owned(),
            wins,
            played: wins + 1,
        }
    }

    fn board(game: GameKind, entries: Vec<LeaderboardEntry>) -> Leaderboard {
        Leaderboard { game, entries }
    }

    #[test]
    fn an_entry_line_has_rank_name_and_wins() {
        assert_eq!(format_entry(&entry(1, "Alex", 42)), "§e1. §fAlex §7— §a42");
        assert_eq!(format_entry(&entry(10, "Bob", 3)), "§e10. §fBob §7— §a3");
    }

    #[test]
    fn a_name_cannot_inject_colour_codes_or_new_lines() {
        assert_eq!(
            format_entry(&entry(2, "§cEv\nil", 1)),
            "§e2. §fcEvil §7— §a1"
        );
    }

    fn lines_of(text: &str) -> Vec<&str> {
        text.split('\n').collect()
    }

    #[test]
    fn a_board_has_a_title_the_game_the_entries_and_a_footer() {
        let entries = vec![entry(1, "Alex", 42), entry(2, "Bob", 7)];
        let text = format_board(
            GameKind::SheepWars,
            Some(&board(GameKind::SheepWars, entries)),
        );
        let lines = lines_of(&text);
        assert_eq!(lines[0], "§6§lMEILLEURS JOUEURS");
        assert_eq!(lines[1], "§dSheepWars §7· victoires");
        assert_eq!(lines[2], "§e1. §fAlex §7— §a42");
        assert_eq!(lines[3], "§e2. §fBob §7— §a7");
        assert_eq!(lines[4], " ");
        assert_eq!(lines[BOARD_LINES - 1], "§8Clic droit : changer de jeu");
    }

    #[test]
    fn the_board_is_always_as_tall_whatever_the_number_of_players() {
        for count in [0, 1, 9, 10, 15] {
            let entries: Vec<_> = (1..=count).map(|r| entry(r, "P", 1)).collect();
            for game in GameKind::ALL {
                let text = format_board(game, Some(&board(game, entries.clone())));
                assert_eq!(lines_of(&text).len(), BOARD_LINES, "{count} players");
            }
        }
        assert_eq!(
            lines_of(&format_board(GameKind::SheepWars, None)).len(),
            BOARD_LINES
        );
    }

    #[test]
    fn towerwars_uses_its_own_colour_and_name() {
        let text = format_board(
            GameKind::TowerWars,
            Some(&board(GameKind::TowerWars, vec![])),
        );
        assert!(text.contains("§bTowerWars §7· victoires"), "{text}");
    }

    #[test]
    fn deacoudre_uses_its_own_colour_and_name() {
        let text = format_board(
            GameKind::DeACoudre,
            Some(&board(GameKind::DeACoudre, vec![])),
        );
        assert!(text.contains("§6Dé à coudre §7· victoires"), "{text}");
    }

    #[test]
    fn a_board_never_shows_more_than_ten_ranks() {
        let entries = (1..=15).map(|r| entry(r, "P", 1)).collect();
        let text = format_board(
            GameKind::SheepWars,
            Some(&board(GameKind::SheepWars, entries)),
        );
        assert!(text.contains("§e10. "));
        assert!(!text.contains("§e11. "));
    }

    #[test]
    fn nobody_has_won_yet() {
        let text = format_board(
            GameKind::SheepWars,
            Some(&board(GameKind::SheepWars, vec![])),
        );
        assert_eq!(lines_of(&text)[2], "§7Aucune partie gagnée pour l'instant");
    }

    #[test]
    fn data_never_received_reads_unavailable() {
        let text = format_board(GameKind::SheepWars, None);
        assert_eq!(lines_of(&text)[2], "§7Classement indisponible");
    }

    #[test]
    fn chunk_of_a_block_position() {
        assert_eq!(chunk_of(27.5, -84.5), (1, -6));
        assert_eq!(chunk_of(0.0, 0.0), (0, 0));
        assert_eq!(chunk_of(15.9, 16.0), (0, 1));
        assert_eq!(chunk_of(-0.5, -16.5), (-1, -2));
    }

    #[test]
    fn no_position_configured_removes_every_display() {
        let plan = reconcile(false, Some(4), &[4, 9]);
        assert_eq!(
            plan,
            Reconcile {
                remove: vec![4, 9],
                spawn: false
            }
        );
    }

    #[test]
    fn first_start_removes_leftovers_then_spawns_one() {
        let plan = reconcile(true, None, &[7, 8]);
        assert_eq!(
            plan,
            Reconcile {
                remove: vec![7, 8],
                spawn: true
            }
        );
    }

    #[test]
    fn repeated_reloads_keep_the_same_display() {
        let plan = reconcile(true, Some(5), &[5]);
        assert_eq!(
            plan,
            Reconcile {
                remove: vec![],
                spawn: false
            }
        );
    }

    #[test]
    fn a_duplicate_next_to_the_tracked_display_is_removed() {
        let plan = reconcile(true, Some(5), &[3, 5]);
        assert_eq!(
            plan,
            Reconcile {
                remove: vec![3],
                spawn: false
            }
        );
    }

    #[test]
    fn a_display_that_vanished_is_spawned_again() {
        let plan = reconcile(true, Some(5), &[]);
        assert_eq!(
            plan,
            Reconcile {
                remove: vec![],
                spawn: true
            }
        );
    }
}
