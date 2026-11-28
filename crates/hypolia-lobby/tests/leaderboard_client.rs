use std::net::SocketAddr;

use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use hypolia_lobby::orchestrator::{OrchestratorClient, OrchestratorClientError};
use hypolia_protocol::{GameKind, Leaderboard, LeaderboardEntry};
use tokio::net::TcpListener;

#[derive(serde::Deserialize)]
struct Limit {
    limit: u32,
}

async fn serve(app: Router) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

fn client(addr: SocketAddr) -> OrchestratorClient {
    OrchestratorClient::new(format!("http://{addr}"), None)
}

#[tokio::test]
async fn fetch_leaderboard_asks_for_the_game_slug_and_the_limit() {
    let app = Router::new().route(
        "/api/v1/leaderboards/{game}",
        get(
            |Path(game): Path<String>, Query(query): Query<Limit>| async move {
                let game = GameKind::from_slug(&game).expect("known slug");
                Json(Leaderboard {
                    game,
                    entries: vec![LeaderboardEntry {
                        rank: 1,
                        name: format!("limit{}", query.limit),
                        wins: 3,
                        played: 4,
                    }],
                })
            },
        ),
    );
    let addr = serve(app).await;

    let board = client(addr)
        .fetch_leaderboard(GameKind::TowerWars, 10)
        .await
        .expect("board");
    assert_eq!(board.game, GameKind::TowerWars);
    assert_eq!(board.entries[0].name, "limit10");
}

#[tokio::test]
async fn fetch_leaderboard_reports_http_errors_and_unreachable_servers() {
    let app = Router::new().route(
        "/api/v1/leaderboards/{game}",
        get(|| async { StatusCode::SERVICE_UNAVAILABLE }),
    );
    let addr = serve(app).await;
    let err = client(addr)
        .fetch_leaderboard(GameKind::SheepWars, 10)
        .await
        .expect_err("503");
    assert!(matches!(err, OrchestratorClientError::HttpStatus(503)));

    let closed = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let dead = closed.local_addr().expect("addr");
    drop(closed);
    let err = client(dead)
        .fetch_leaderboard(GameKind::SheepWars, 10)
        .await
        .expect_err("unreachable");
    assert!(matches!(err, OrchestratorClientError::Transport(_)));
}
