use hypolia_protocol::FleetStatusSnapshot;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetworkView {
    pub online: u32,
    pub in_game: u32,
    pub lobby_number: Option<usize>,
}

/// Counts the whole network from the orchestrator snapshot. This lobby's own
/// entry lags behind its heartbeat, so its live count replaces it.
pub fn network_view(
    snapshot: Option<&FleetStatusSnapshot>,
    own_server_id: Option<&str>,
    local_online: u32,
) -> NetworkView {
    let Some(snapshot) = snapshot else {
        return NetworkView {
            online: local_online,
            ..NetworkView::default()
        };
    };
    let is_own = |id: &str| own_server_id == Some(id);
    let other_lobbies: u32 = snapshot
        .lobbies
        .iter()
        .filter(|lobby| !is_own(&lobby.server_id))
        .map(|lobby| lobby.player_count)
        .sum();
    let in_game = snapshot.players_in_game();

    let mut lobby_ids: Vec<&str> = snapshot
        .lobbies
        .iter()
        .map(|lobby| lobby.server_id.as_str())
        .collect();
    lobby_ids.sort_unstable();
    let lobby_number = own_server_id
        .and_then(|own| lobby_ids.iter().position(|id| *id == own))
        .map(|index| index + 1);

    NetworkView {
        online: other_lobbies + local_online + in_game,
        in_game,
        lobby_number,
    }
}

#[cfg(test)]
mod tests {
    use hypolia_protocol::{ServerKind, ServerPhase, ServerSummary};

    use super::*;

    fn server(id: &str, kind: ServerKind, players: u32) -> ServerSummary {
        ServerSummary {
            server_id: id.to_owned(),
            pod_name: None,
            address: "10.0.0.1:25565".to_owned(),
            phase: ServerPhase::Waiting,
            player_count: players,
            max_players: 100,
            ready: true,
            heartbeat_age_secs: 1,
            age_secs: 10,
            version: None,
            outdated: false,
            kind,
            draining: false,
        }
    }

    fn snapshot(lobbies: Vec<ServerSummary>, servers: Vec<ServerSummary>) -> FleetStatusSnapshot {
        FleetStatusSnapshot {
            queue_len: 0,
            min_players: 1,
            max_players_per_server: 16,
            min_replicas: 0,
            max_replicas: 4,
            standby_servers: 0,
            ready_servers: 0,
            in_game_servers: 0,
            starting_servers: 0,
            queued_players: Vec::new(),
            servers,
            orchestrator_version: None,
            lobbies,
            proxy_version: None,
            games: Vec::new(),
        }
    }

    #[test]
    fn one_player_on_each_lobby_shows_two_online_on_both() {
        let snap = snapshot(
            vec![
                server("lobby-a", ServerKind::Lobby, 1),
                server("lobby-b", ServerKind::Lobby, 1),
            ],
            Vec::new(),
        );
        assert_eq!(network_view(Some(&snap), Some("lobby-a"), 1).online, 2);
        assert_eq!(network_view(Some(&snap), Some("lobby-b"), 1).online, 2);
    }

    #[test]
    fn the_live_local_count_replaces_the_lagging_heartbeat() {
        let snap = snapshot(
            vec![
                server("lobby-a", ServerKind::Lobby, 0),
                server("lobby-b", ServerKind::Lobby, 2),
            ],
            vec![server("sw-1", ServerKind::SheepWars, 4)],
        );
        let view = network_view(Some(&snap), Some("lobby-a"), 3);
        assert_eq!(view.online, 3 + 2 + 4);
        assert_eq!(view.in_game, 4);
    }

    #[test]
    fn a_lobby_missing_from_the_snapshot_still_counts_its_players() {
        let snap = snapshot(vec![server("lobby-b", ServerKind::Lobby, 2)], Vec::new());
        let view = network_view(Some(&snap), Some("lobby-a"), 1);
        assert_eq!(view.online, 3);
        assert_eq!(view.lobby_number, None);
    }

    #[test]
    fn lobbies_are_numbered_by_their_sorted_server_id() {
        let snap = snapshot(
            vec![
                server("lobby-b", ServerKind::Lobby, 0),
                server("lobby-a", ServerKind::Lobby, 0),
            ],
            Vec::new(),
        );
        assert_eq!(
            network_view(Some(&snap), Some("lobby-b"), 0).lobby_number,
            Some(2)
        );
    }

    #[test]
    fn without_an_orchestrator_only_this_lobby_counts() {
        let view = network_view(None, None, 5);
        assert_eq!(view.online, 5);
        assert_eq!(view.in_game, 0);
        assert_eq!(view.lobby_number, None);
    }
}
