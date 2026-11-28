use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use hypolia_protocol::{GameKind, Leaderboard, LeaderboardEntry};
use rspigot_plugin::{Billboard, DisplayProps, Server, TextDisplay, World};
use tokio::sync::Mutex;
use tokio::time::{Instant, MissedTickBehavior, interval_at};

use crate::config::LayoutStore;
use crate::orchestrator::OrchestratorClient;
use crate::progress_menus::game_label;

pub const BOARD_SIZE: u32 = 10;
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
pub const ROTATE_INTERVAL: Duration = Duration::from_secs(8);
const META_KEY: &str = "hypolia:leaderboard";

const fn game_color(game: GameKind) -> &'static str {
    match game {
        GameKind::SheepWars => "§d",
        GameKind::TowerWars => "§b",
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

pub struct LeaderboardHud {
    layout: Arc<LayoutStore>,
    orchestrator: Option<Arc<OrchestratorClient>>,
    cache: RwLock<BTreeMap<GameKind, Leaderboard>>,
    shown: AtomicUsize,
    display: Mutex<Option<i32>>,
}

impl LeaderboardHud {
    pub fn new(layout: Arc<LayoutStore>, orchestrator: Option<Arc<OrchestratorClient>>) -> Self {
        Self {
            layout,
            orchestrator,
            cache: RwLock::new(BTreeMap::new()),
            shown: AtomicUsize::new(0),
            display: Mutex::new(None),
        }
    }

    pub fn shown_game(&self) -> GameKind {
        GameKind::ALL[self.shown.load(Ordering::Relaxed) % GameKind::ALL.len()]
    }

    fn advance(&self) {
        self.shown.fetch_add(1, Ordering::Relaxed);
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
        let mut tracked = self.display.lock().await;
        let found = match world.entities.with_meta(META_KEY).await {
            Ok(entities) => entities,
            Err(err) => {
                tracing::warn!("Impossible de lister les tableaux de classement : {err}");
                return;
            }
        };
        let ids: Vec<i32> = found.iter().map(|e| e.entity_id).collect();
        let plan = reconcile(spot.is_some(), *tracked, &ids);
        for entity in found.iter().filter(|e| plan.remove.contains(&e.entity_id)) {
            entity.remove();
        }
        let Some(spot) = spot else {
            *tracked = None;
            return;
        };
        if plan.spawn {
            match world
                .objects
                .spawn_display(spot.position(), self.spec())
                .await
            {
                Ok(entity) => {
                    entity.meta.set(META_KEY, true);
                    *tracked = Some(entity.entity_id);
                }
                Err(err) => {
                    *tracked = None;
                    tracing::warn!("Impossible de placer le tableau de classement : {err}");
                }
            }
        } else if let Some(entity) = found.iter().find(|e| Some(e.entity_id) == *tracked) {
            entity.teleport(spot.position());
            entity.display.edit(self.spec());
        }
    }

    async fn redraw(&self, world: &World) {
        let tracked = self.display.lock().await;
        let Some(id) = *tracked else {
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
        let mut rotate = interval_at(Instant::now() + ROTATE_INTERVAL, ROTATE_INTERVAL);
        refresh.set_missed_tick_behavior(MissedTickBehavior::Delay);
        rotate.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = refresh.tick() => {
                    if self.layout.snapshot().leaderboard.is_some() {
                        self.refresh().await;
                        self.redraw(&server.world).await;
                    }
                }
                _ = rotate.tick() => {
                    self.advance();
                    self.redraw(&server.world).await;
                }
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

    #[test]
    fn a_board_has_a_title_the_game_and_one_line_per_entry() {
        let entries = vec![entry(1, "Alex", 42), entry(2, "Bob", 7)];
        let text = format_board(
            GameKind::SheepWars,
            Some(&board(GameKind::SheepWars, entries)),
        );
        assert_eq!(
            text,
            "§6§lMEILLEURS JOUEURS\n§dSheepWars §7· victoires\n§e1. §fAlex §7— §a42\n§e2. §fBob §7— §a7"
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
    fn a_board_never_shows_more_than_ten_lines() {
        let entries = (1..=15).map(|r| entry(r, "P", 1)).collect();
        let text = format_board(
            GameKind::SheepWars,
            Some(&board(GameKind::SheepWars, entries)),
        );
        assert_eq!(text.lines().count(), 2 + 10);
    }

    #[test]
    fn nobody_has_won_yet() {
        let text = format_board(
            GameKind::SheepWars,
            Some(&board(GameKind::SheepWars, vec![])),
        );
        assert!(text.ends_with("\n§7Aucune partie gagnée pour l'instant"));
    }

    #[test]
    fn data_never_received_reads_unavailable() {
        let text = format_board(GameKind::SheepWars, None);
        assert!(text.ends_with("\n§7Classement indisponible"));
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
