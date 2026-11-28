use std::collections::HashMap;
use std::sync::RwLock;

use rspigot_plugin::{
    Criteria, DisplaySlot, Error, Objective, Player, Scoreboard, Server, TabList, TeamColor, Uuid,
};

use crate::auth::{AuthManager, Rank};
use crate::profile::{ProfileCache, level_prefix};

const TEAM_DEFS: [(&str, &str, &str, TeamColor); 7] = [
    ("00_admin_fk", "§c§lADMIN §f", " §a✔", TeamColor::Red),
    ("01_admin", "§c§lADMIN §f", "", TeamColor::Red),
    ("10_mod", "§9§lMOD §f", " §a✔", TeamColor::Blue),
    ("20_dev", "§b§lDEV §f", " §a✔", TeamColor::Aqua),
    ("30_vip", "§6§lVIP §f", " §a✔", TeamColor::Gold),
    ("40_member", "§a§lFK §f", " §a✔", TeamColor::Green),
    ("90_guest", "§7", "", TeamColor::Gray),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TabKey {
    rank: Rank,
    verified: bool,
    level: Option<u32>,
}

fn team_def(team: &str) -> Option<&'static (&'static str, &'static str, &'static str, TeamColor)> {
    TEAM_DEFS.iter().find(|(id, ..)| *id == team)
}

fn player_team_id(name: &str, rank: Rank, verified: bool) -> String {
    format!("{}_{name}", rank.team_name(verified))
}

fn tab_prefix(rank: Rank, verified: bool, level: Option<u32>) -> String {
    let base = team_def(rank.team_name(verified)).map_or("", |(_, prefix, _, _)| *prefix);
    format!("{}{base}", level_prefix(level))
}

fn install_player_team(board: &Scoreboard, name: &str, key: TabKey) {
    let id = player_team_id(name, key.rank, key.verified);
    let team = board.register_team(id.as_str());
    let (suffix, color) = team_def(key.rank.team_name(key.verified))
        .map_or(("", None), |(_, _, suffix, color)| (*suffix, Some(*color)));
    team.set_prefix(tab_prefix(key.rank, key.verified, key.level));
    team.set_suffix(suffix);
    team.set_color(color);
    team.add_entry(name);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HubMetrics {
    /// Players on the whole network, not only this lobby.
    pub online: u32,
    pub in_game: u32,
    pub lobby_number: Option<usize>,
    pub tps_1m: f64,
    pub agents_working: usize,
    pub agents_total: usize,
}

#[derive(Debug, Clone)]
pub struct PlayerHudInput {
    pub rank: Rank,
    pub ferriskey_username: Option<String>,
    pub ping_ms: i32,
    pub metrics: HubMetrics,
    pub network_level: Option<u32>,
    pub level_percent: Option<u32>,
    pub coins: Option<u64>,
}

pub fn build_sidebar_lines(input: &PlayerHudInput) -> Vec<String> {
    let fk_line = match &input.ferriskey_username {
        Some(user) => {
            let short: String = user.chars().take(14).collect();
            format!("§fFerrisKey : §a✔ {short}")
        }
        None => "§fFerrisKey : §e/login".to_owned(),
    };
    let ping = input.ping_ms.max(0);

    let mut lines = vec![
        "§1".to_owned(),
        format!("§fGrade : {}", input.rank.sidebar_label()),
        fk_line,
    ];
    if let Some(level) = input.network_level {
        let percent = input.level_percent.unwrap_or(0);
        lines.push(format!("§fNiveau : §b{level} §7({percent}%)"));
    }
    if let Some(coins) = input.coins {
        lines.push(format!("§fCoins : §6{coins}"));
    }
    lines.extend([
        "§2".to_owned(),
        format!("§fJoueurs : §a{}", input.metrics.online),
        format!("§fEn jeu : §d{}", input.metrics.in_game),
    ]);
    if let Some(number) = input.metrics.lobby_number {
        lines.push(format!("§fLobby : §e#{number}"));
    }
    lines.extend([
        format!(
            "§fCapitainerie : §b{}§7/§f{}",
            input.metrics.agents_working, input.metrics.agents_total
        ),
        "§3".to_owned(),
        format!("§fPing : §a{ping}ms"),
        "§4".to_owned(),
        "§6play.ferrislabs.fr".to_owned(),
    ]);
    lines
}

pub fn build_tab_list(input: &PlayerHudInput) -> TabList {
    let fk_status = match &input.ferriskey_username {
        Some(user) => format!("§a✔ {user}"),
        None => "§eNon lié (/login)".to_owned(),
    };
    let tps = input.metrics.tps_1m.clamp(0.0, 20.0);
    let ping = input.ping_ms.max(0);

    let header = format!(
        "\n§6§l✦ HYPOLIA §8• §d§lHUB MINI-JEUX ✦\n§7Île Nordique §8• §fJoueurs : §a{}\n",
        input.metrics.online
    );
    let footer = format!(
        "\n§7Grade : {} §8• §7Compte : {fk_status}\n§7Ping : §a{ping} ms §8• §7TPS : §a{tps:.1} §8• §7En jeu : §d{}\n§6/menu §8• §d/sheepwars §8• §b/towerwars §8• §6/deacoudre §8• §b/gadgets §8• §b/fly §8• §e/options\n",
        input.rank.sidebar_label(),
        input.metrics.in_game
    );
    TabList::new(header, footer)
}

struct BoardEntry {
    board: Scoreboard,
    objective: Objective,
    last_lines: Vec<String>,
    last_tab: Option<TabList>,
}

#[derive(Default)]
pub struct HudManager {
    boards: RwLock<HashMap<Uuid, BoardEntry>>,
    team_by_player: RwLock<HashMap<String, TabKey>>,
}

impl HudManager {
    pub async fn attach_player(&self, server: &Server, player: &Player) -> Result<(), Error> {
        self.detach_player(player.uuid, &player.name);

        let board = server.scoreboard_manager.new_scoreboard().await?;
        let objective =
            board.register_objective("fc_hub", &Criteria::dummy(), "§6§lHYPOLIA §8• §dHUB");
        board.set_display_slot(DisplaySlot::Sidebar, Some(&objective));

        // Populate existing players' teams on this fresh board so TabList renders immediately.
        let existing_teams: Vec<(String, TabKey)> = self
            .team_by_player
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(name, key)| (name.clone(), *key))
            .collect();
        for (name, key) in existing_teams {
            install_player_team(&board, &name, key);
        }

        player.scoreboard.set(&board);
        self.boards
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                player.uuid,
                BoardEntry {
                    board,
                    objective,
                    last_lines: Vec::new(),
                    last_tab: None,
                },
            );
        Ok(())
    }

    pub fn detach_player(&self, uuid: Uuid, player_name: &str) {
        let removed_board = self
            .boards
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&uuid);
        if let Some(entry) = removed_board {
            entry.board.discard();
        }

        let old_team = self
            .team_by_player
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(player_name);
        if let Some(key) = old_team {
            let id = player_team_id(player_name, key.rank, key.verified);
            let boards = self.boards.read().unwrap_or_else(|e| e.into_inner());
            for entry in boards.values() {
                entry.board.team(id.as_str()).unregister();
            }
        }
    }

    pub fn sync_player_team(
        &self,
        player_name: &str,
        rank: Rank,
        verified: bool,
        level: Option<u32>,
    ) {
        let key = TabKey {
            rank,
            verified,
            level,
        };
        let previous = self
            .team_by_player
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(player_name.to_owned(), key);

        if previous == Some(key) {
            return;
        }

        let boards = self.boards.read().unwrap_or_else(|e| e.into_inner());
        match previous {
            Some(old) if old.rank.team_name(old.verified) == rank.team_name(verified) => {
                let id = player_team_id(player_name, rank, verified);
                let prefix = tab_prefix(rank, verified, level);
                for entry in boards.values() {
                    entry.board.team(id.as_str()).set_prefix(prefix.as_str());
                }
            }
            Some(old) => {
                let old_id = player_team_id(player_name, old.rank, old.verified);
                for entry in boards.values() {
                    entry.board.team(old_id.as_str()).unregister();
                    install_player_team(&entry.board, player_name, key);
                }
            }
            None => {
                for entry in boards.values() {
                    install_player_team(&entry.board, player_name, key);
                }
            }
        }
    }

    pub fn refresh_player(&self, player: &Player, input: &PlayerHudInput) {
        self.sync_player_team(
            &player.name,
            input.rank,
            input.ferriskey_username.is_some(),
            input.network_level,
        );

        let lines = build_sidebar_lines(input);
        let tab = build_tab_list(input);

        let mut boards = self.boards.write().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = boards.get_mut(&player.uuid) {
            let total = lines.len();
            if entry.last_lines.len() != total {
                for old in &entry.last_lines {
                    entry.board.reset_scores(old);
                }
                for (idx, line) in lines.iter().enumerate() {
                    let score = (total - idx) as i32;
                    entry.objective.score(line).set(score);
                }
            } else {
                for (idx, (old, new)) in entry.last_lines.iter().zip(&lines).enumerate() {
                    if old != new {
                        let score = (total - idx) as i32;
                        entry.board.reset_scores(old);
                        entry.objective.score(new).set(score);
                    }
                }
            }
            entry.last_lines = lines;

            if entry.last_tab.as_ref() != Some(&tab) {
                player.tab_list.set(tab.clone());
                entry.last_tab = Some(tab);
            }
        } else {
            player.tab_list.set(tab);
        }
    }

    pub async fn refresh_all(
        &self,
        server: &Server,
        auth: &AuthManager,
        profiles: &ProfileCache,
        metrics: HubMetrics,
    ) {
        let Ok(players) = server.get_players().await else {
            return;
        };
        for player in players {
            let (ping_ms, is_op) = match player.get_state().await {
                Ok(state) => (state.ping, state.op),
                Err(_) => (0, false),
            };
            let claims = auth.claims_of(player.uuid);
            let has_admin = player.permissions.has("hypolia.admin");
            let rank = Rank::resolve(claims.as_ref(), is_op, has_admin);
            let input = PlayerHudInput {
                rank,
                ferriskey_username: claims.map(|c| c.username),
                ping_ms,
                metrics,
                network_level: profiles.level(player.uuid),
                level_percent: profiles.level_percent(player.uuid),
                coins: profiles.coins(player.uuid),
            };
            self.refresh_player(&player, &input);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn sidebar_lines_are_all_unique_and_reflect_ferriskey_status() {
        let input = PlayerHudInput {
            rank: Rank::Dev,
            ferriskey_username: Some("nathael".to_owned()),
            ping_ms: 18,
            metrics: HubMetrics {
                online: 12,
                in_game: 5,
                lobby_number: Some(2),
                tps_1m: 20.0,
                agents_working: 2,
                agents_total: 4,
            },
            network_level: Some(12),
            level_percent: Some(45),
            coins: Some(340),
        };
        let lines = build_sidebar_lines(&input);
        let unique: HashSet<&String> = lines.iter().collect();
        assert_eq!(
            lines.len(),
            unique.len(),
            "each scoreboard entry must be unique"
        );
        assert!(lines.iter().any(|l| l.contains("✔ nathael")));
        assert!(lines.iter().any(|l| l == "§fJoueurs : §a12"));
        assert!(lines.iter().any(|l| l == "§fEn jeu : §d5"));
        assert!(lines.iter().any(|l| l == "§fLobby : §e#2"));
        assert!(lines.iter().any(|l| l == "§fNiveau : §b12 §7(45%)"));
        assert!(lines.iter().any(|l| l == "§fCoins : §6340"));
        assert!(!lines.iter().any(|l| l.contains("prêt(s)")));

        let tab = build_tab_list(&input);
        assert!(tab.header.contains("HYPOLIA"));
        assert!(tab.footer.contains("✔ nathael"));
    }

    #[test]
    fn sidebar_shows_no_coins_line_until_a_profile_is_known() {
        let input = PlayerHudInput {
            rank: Rank::Guest,
            ferriskey_username: None,
            ping_ms: 0,
            metrics: HubMetrics::default(),
            network_level: None,
            level_percent: None,
            coins: None,
        };
        assert!(
            !build_sidebar_lines(&input)
                .iter()
                .any(|l| l.contains("Coins"))
        );
    }

    #[test]
    fn tab_prefix_puts_the_network_level_before_the_rank_prefix() {
        assert_eq!(tab_prefix(Rank::Vip, true, Some(12)), "§7[12✫] §6§lVIP §f");
        assert_eq!(tab_prefix(Rank::Guest, false, None), "§7");
    }

    #[test]
    fn player_team_ids_sort_by_rank_then_name() {
        let admin = player_team_id("Zed", Rank::Admin, true);
        let member = player_team_id("Abe", Rank::Member, true);
        assert!(admin < member);
        assert_eq!(admin, "00_admin_fk_Zed");
    }
}
