use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use axum::{Json, Router};
use hypolia_lobby::heartbeat::{FAILURE_LOG_EVERY, FailureLog, HeartbeatReporter};
use hypolia_lobby::lifecycle::{ShutdownOutcome, graceful_shutdown};
use hypolia_lobby::orchestrator::OrchestratorClient;
use hypolia_lobby::storage::WriteGate;
use hypolia_protocol::{HeartbeatRequest, ServerKind, ServerPhase};
use tokio::net::TcpListener;
use uuid::Uuid;

type Received = Arc<Mutex<Vec<(Option<String>, HeartbeatRequest)>>>;

async fn serve() -> (SocketAddr, Received) {
    let received: Received = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route(
            "/api/v1/servers/heartbeat",
            post(
                |State(received): State<Received>,
                 headers: HeaderMap,
                 Json(body): Json<HeartbeatRequest>| async move {
                    let auth = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    received.lock().expect("lock").push((auth, body));
                },
            ),
        )
        .with_state(Arc::clone(&received));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, received)
}

fn reporter(addr: SocketAddr) -> HeartbeatReporter {
    HeartbeatReporter::new(
        OrchestratorClient::new(format!("http://{addr}"), Some("tok".to_owned())),
        "hypolia-lobby-7d9".to_owned(),
        Some("10.1.2.3:25565".to_owned()),
        Some("abc123".to_owned()),
    )
}

#[tokio::test]
async fn lobby_heartbeat_carries_identity_players_and_version() {
    let (addr, received) = serve().await;
    let reporter = reporter(addr);
    let players = vec![Uuid::new_v4(), Uuid::new_v4()];
    reporter.observe(80, players.clone());

    reporter.beat(ServerPhase::Waiting).await.expect("beat");

    let got = received.lock().expect("lock");
    let (auth, body) = &got[0];
    assert_eq!(auth.as_deref(), Some("Bearer tok"));
    assert_eq!(body.kind, ServerKind::Lobby);
    assert_eq!(body.server_id, "hypolia-lobby-7d9");
    assert_eq!(body.address.as_deref(), Some("10.1.2.3:25565"));
    assert_eq!(body.phase, ServerPhase::Waiting);
    assert_eq!(body.player_count, 2);
    assert_eq!(body.max_players, 80);
    assert_eq!(body.player_uuids, players);
    assert_eq!(body.version.as_deref(), Some("abc123"));
}

#[test]
fn failure_log_warns_once_per_30_seconds_and_reports_recovery() {
    let mut log = FailureLog::default();
    let start = Instant::now();
    assert!(log.on_failure(start));
    assert!(!log.on_failure(start + Duration::from_secs(2)));
    assert!(!log.on_failure(start + Duration::from_secs(29)));
    assert!(log.on_failure(start + FAILURE_LOG_EVERY));
    assert!(log.on_success());
    assert!(!log.on_success());
    assert!(log.on_failure(start + FAILURE_LOG_EVERY + Duration::from_secs(1)));
}

#[tokio::test]
async fn plc_8_graceful_shutdown_sends_a_last_heartbeat_and_closes_writes() {
    let (addr, received) = serve().await;
    let reporter = reporter(addr);
    reporter.observe(100, vec![Uuid::new_v4()]);
    let gate = WriteGate::default();
    let in_flight = gate.try_enter().expect("write in flight");

    let started = Instant::now();
    let shutdown = graceful_shutdown(&gate, Some(&reporter), Duration::from_secs(4));
    tokio::pin!(shutdown);
    tokio::select! {
        _ = &mut shutdown => panic!("shutdown must wait for the in-flight write"),
        () = tokio::time::sleep(Duration::from_millis(200)) => {}
    }
    drop(in_flight);
    assert_eq!(shutdown.await, ShutdownOutcome::Clean);
    assert!(started.elapsed() < Duration::from_secs(5));

    assert!(gate.try_enter().is_err(), "new writes are refused");
    let got = received.lock().expect("lock");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].1.phase, ServerPhase::Ending);
    assert_eq!(got[0].1.kind, ServerKind::Lobby);
    assert_eq!(got[0].1.player_count, 1);
}

#[tokio::test]
async fn graceful_shutdown_gives_up_when_a_write_never_finishes() {
    let gate = WriteGate::default();
    let _stuck = gate.try_enter().expect("write in flight");
    let outcome = graceful_shutdown(&gate, None, Duration::from_millis(200)).await;
    assert_eq!(outcome, ShutdownOutcome::TimedOut);
}
