use hypolia_protocol::{ServerKind, ServerPhase, ServerSummary};
use rspigot_plugin::{CustomInventory, ItemStack, Material};

pub const SERVERS_ROWS: u8 = 6;
pub const SERVERS_SLOT_HEADER: u16 = 4;
pub const SERVERS_FIRST_SLOT: u16 = 9;
pub const SERVERS_MAX: usize = 36;

pub fn short_version(version: &str) -> String {
    if is_full_sha(version) {
        return version[..7].to_owned();
    }
    if let Some(rest) = version.strip_prefix("pr-")
        && let Some((number, sha)) = rest.split_once('-')
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
        && is_full_sha(sha)
    {
        return format!("pr-{number}-{}", &sha[..7]);
    }
    version.to_owned()
}

fn is_full_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn short_version_or_unknown(version: Option<&str>) -> String {
    version.map_or_else(|| "?".to_owned(), short_version)
}

pub fn format_age(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

pub const fn game_label(kind: ServerKind) -> &'static str {
    match kind {
        ServerKind::Lobby => "Lobby",
        ServerKind::SheepWars => "SheepWars",
        ServerKind::TowerWars => "TowerWars",
        ServerKind::DeACoudre => "Dé à coudre",
    }
}

pub fn phase_style(phase: ServerPhase) -> (Material, &'static str) {
    match phase {
        ServerPhase::Starting => (Material::LightBlueWool, "Démarrage"),
        ServerPhase::Waiting => (Material::LimeWool, "En attente"),
        ServerPhase::Countdown => (Material::YellowWool, "Compte à rebours"),
        ServerPhase::InGame => (Material::OrangeWool, "En jeu"),
        ServerPhase::Ending => (Material::GrayWool, "Fin de partie"),
    }
}

pub fn decode_servers_slot(slot: i32, server_count: usize) -> Option<usize> {
    let slot = u16::try_from(slot).ok()?;
    let index = usize::from(slot.checked_sub(SERVERS_FIRST_SLOT)?);
    (index < server_count.min(SERVERS_MAX)).then_some(index)
}

pub fn sorted_server_ids(
    servers: &[ServerSummary],
    lobbies: &[ServerSummary],
) -> Vec<Option<String>> {
    ordered_entries(servers, lobbies)
        .into_iter()
        .map(|(server, is_lobby)| (!is_lobby).then(|| server.server_id.clone()))
        .collect()
}

fn ordered_entries<'a>(
    servers: &'a [ServerSummary],
    lobbies: &'a [ServerSummary],
) -> Vec<(&'a ServerSummary, bool)> {
    let mut games: Vec<&ServerSummary> = servers.iter().collect();
    games.sort_by(|a, b| a.server_id.cmp(&b.server_id));
    let mut instances: Vec<&ServerSummary> = lobbies.iter().collect();
    instances.sort_by(|a, b| a.server_id.cmp(&b.server_id));
    games
        .into_iter()
        .map(|server| (server, false))
        .chain(instances.into_iter().map(|server| (server, true)))
        .take(SERVERS_MAX)
        .collect()
}

pub fn lobby_style(server: &ServerSummary) -> (Material, &'static str) {
    if server.draining {
        (Material::SoulLantern, "Drain")
    } else if server.ready {
        (Material::Lantern, "Prêt")
    } else {
        (Material::Lantern, "Démarrage")
    }
}

fn lobby_item(server: &ServerSummary) -> ItemStack {
    let (material, label) = lobby_style(server);
    let color = if server.draining { "§6" } else { "§b" };
    let mut lore = vec![
        "§7Instance de lobby".to_owned(),
        format!("§7État {color}{label}"),
        format!("§7Joueurs §f{}/{}", server.player_count, server.max_players),
        format!(
            "§7Version §f{}",
            short_version_or_unknown(server.version.as_deref())
        ),
        format!("§7Âge §f{}", format_age(server.age_secs)),
    ];
    if server.draining {
        lore.push("§6Les joueurs sont transférés vers un autre lobby".to_owned());
    }
    if server.outdated {
        lore.push("§cAncienne version".to_owned());
    }
    ItemStack::new(material, 1)
        .name(format!("{color}{}", server.server_id))
        .lore(lore)
}

fn server_item(server: &ServerSummary) -> ItemStack {
    let (phase_material, label) = phase_style(server.phase);
    let material = if server.outdated {
        Material::RedWool
    } else {
        phase_material
    };
    let name = if server.outdated {
        format!("§c{}", server.server_id)
    } else {
        format!("§f{}", server.server_id)
    };
    let mut lore = vec![
        format!("§7Jeu §f{}", game_label(server.kind)),
        format!("§7{label}"),
        format!("§7Joueurs §f{}/{}", server.player_count, server.max_players),
        format!(
            "§7Version §f{}",
            short_version_or_unknown(server.version.as_deref())
        ),
        format!("§7Âge §f{}", format_age(server.age_secs)),
    ];
    if server.outdated {
        lore.push("§cAncienne version".to_owned());
    }
    if server.ready {
        lore.push("§eClic : observer".to_owned());
    }
    ItemStack::new(material, 1).name(name).lore(lore)
}

pub fn populate_servers_menu(
    inv: &CustomInventory,
    lobby_version: Option<&str>,
    orchestrator_version: Option<&str>,
    proxy_version: Option<&str>,
    servers: &[ServerSummary],
    lobbies: &[ServerSummary],
) {
    inv.set(
        SERVERS_SLOT_HEADER,
        Some(
            ItemStack::new(Material::Book, 1)
                .name("§dVersions déployées")
                .lore(header_lore(
                    lobby_version,
                    orchestrator_version,
                    proxy_version,
                )),
        ),
    );
    for (index, (server, is_lobby)) in ordered_entries(servers, lobbies).into_iter().enumerate() {
        let slot = SERVERS_FIRST_SLOT + u16::try_from(index).unwrap_or(0);
        let item = if is_lobby {
            lobby_item(server)
        } else {
            server_item(server)
        };
        inv.set(slot, Some(item));
    }
}

pub fn header_lore(
    lobby_version: Option<&str>,
    orchestrator_version: Option<&str>,
    proxy_version: Option<&str>,
) -> Vec<String> {
    vec![
        format!("§7Lobby §f{}", short_version_or_unknown(lobby_version)),
        format!(
            "§7Orchestrateur §f{}",
            short_version_or_unknown(orchestrator_version)
        ),
        format!("§7Proxy §f{}", short_version_or_unknown(proxy_version)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "221bc38da1571c58215910983e4ab060145106f9";

    #[test]
    fn short_version_handles_sha_pr_and_other() {
        assert_eq!(short_version(SHA), "221bc38");
        assert_eq!(short_version(&format!("pr-12-{SHA}")), "pr-12-221bc38");
        assert_eq!(short_version("dev"), "dev");
        assert_eq!(short_version("pr-x-abc"), "pr-x-abc");
        assert_eq!(short_version(&SHA[..39]), &SHA[..39]);
    }

    #[test]
    fn unknown_version_is_question_mark() {
        assert_eq!(short_version_or_unknown(None), "?");
        assert_eq!(short_version_or_unknown(Some(SHA)), "221bc38");
    }

    #[test]
    fn age_formats() {
        assert_eq!(format_age(42), "42s");
        assert_eq!(format_age(185), "3m 05s");
        assert_eq!(format_age(2 * 3600 + 7 * 60 + 30), "2h 07m");
        assert_eq!(format_age(0), "0s");
        assert_eq!(format_age(3600), "1h 00m");
    }

    #[test]
    fn phase_styles() {
        assert_eq!(
            phase_style(ServerPhase::Starting),
            (Material::LightBlueWool, "Démarrage")
        );
        assert_eq!(
            phase_style(ServerPhase::Waiting),
            (Material::LimeWool, "En attente")
        );
        assert_eq!(
            phase_style(ServerPhase::Countdown),
            (Material::YellowWool, "Compte à rebours")
        );
        assert_eq!(
            phase_style(ServerPhase::InGame),
            (Material::OrangeWool, "En jeu")
        );
        assert_eq!(
            phase_style(ServerPhase::Ending),
            (Material::GrayWool, "Fin de partie")
        );
    }

    #[test]
    fn slot_decoding() {
        assert_eq!(decode_servers_slot(9, 3), Some(0));
        assert_eq!(decode_servers_slot(11, 3), Some(2));
        assert_eq!(decode_servers_slot(12, 3), None);
        assert_eq!(decode_servers_slot(4, 3), None);
        assert_eq!(decode_servers_slot(-1, 3), None);
        assert_eq!(decode_servers_slot(45, 100), None);
        assert_eq!(decode_servers_slot(44, 100), Some(35));
    }

    fn summary(id: &str) -> ServerSummary {
        ServerSummary {
            server_id: id.to_owned(),
            pod_name: None,
            address: String::new(),
            phase: ServerPhase::Waiting,
            player_count: 0,
            max_players: 10,
            ready: true,
            heartbeat_age_secs: 0,
            age_secs: 0,
            version: None,
            outdated: false,
            kind: Default::default(),
            draining: false,
        }
    }

    #[test]
    fn each_game_server_names_its_game() {
        assert_eq!(game_label(ServerKind::SheepWars), "SheepWars");
        assert_eq!(game_label(ServerKind::TowerWars), "TowerWars");
        assert_eq!(game_label(ServerKind::DeACoudre), "Dé à coudre");
        assert_eq!(game_label(ServerKind::Lobby), "Lobby");
    }

    #[test]
    fn ids_sorted_and_capped() {
        let servers: Vec<ServerSummary> = (0..50)
            .rev()
            .map(|i| summary(&format!("s{i:02}")))
            .collect();
        let ids = sorted_server_ids(&servers, &[]);
        assert_eq!(ids.len(), SERVERS_MAX);
        assert_eq!(ids[0].as_deref(), Some("s00"));
        assert_eq!(ids[35].as_deref(), Some("s35"));
    }

    #[test]
    fn lobbies_follow_games_and_are_not_clickable() {
        let servers = vec![summary("sw-b"), summary("sw-a")];
        let lobbies = vec![summary("lobby-1"), summary("lobby-0")];
        let ids = sorted_server_ids(&servers, &lobbies);
        assert_eq!(
            ids,
            vec![Some("sw-a".to_owned()), Some("sw-b".to_owned()), None, None]
        );
        let order: Vec<&str> = ordered_entries(&servers, &lobbies)
            .into_iter()
            .map(|(s, _)| s.server_id.as_str())
            .collect();
        assert_eq!(order, ["sw-a", "sw-b", "lobby-0", "lobby-1"]);
    }

    #[test]
    fn lobby_items_have_a_distinct_material_and_a_drain_state() {
        let ready = summary("lobby-0");
        let mut draining = summary("lobby-1");
        draining.draining = true;
        assert_eq!(lobby_style(&ready), (Material::Lantern, "Prêt"));
        assert_eq!(lobby_style(&draining), (Material::SoulLantern, "Drain"));
        assert_ne!(lobby_style(&ready).0, phase_style(ready.phase).0);
    }

    #[test]
    fn header_shows_lobby_orchestrator_and_proxy_versions() {
        let lore = header_lore(
            Some("dev"),
            Some(SHA),
            Some("pr-3-221bc38da1571c58215910983e4ab060145106f9"),
        );
        assert_eq!(
            lore,
            vec![
                "§7Lobby §fdev".to_owned(),
                "§7Orchestrateur §f221bc38".to_owned(),
                "§7Proxy §fpr-3-221bc38".to_owned(),
            ]
        );
        assert_eq!(header_lore(None, None, None)[2], "§7Proxy §f?");
    }
}
