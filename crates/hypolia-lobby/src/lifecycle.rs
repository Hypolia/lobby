use std::time::Duration;

use tokio::signal::unix::{Signal, SignalKind, signal};

use crate::heartbeat::HeartbeatReporter;
use crate::storage::WriteGate;

pub const SHUTDOWN_BUDGET: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownOutcome {
    Clean,
    TimedOut,
}

pub async fn graceful_shutdown(
    gate: &WriteGate,
    heartbeat: Option<&HeartbeatReporter>,
    budget: Duration,
) -> ShutdownOutcome {
    let flush = async {
        let last_heartbeat = async {
            if let Some(reporter) = heartbeat {
                reporter.send_final().await;
            }
        };
        tokio::join!(gate.close(), last_heartbeat);
    };
    match tokio::time::timeout(budget, flush).await {
        Ok(()) => ShutdownOutcome::Clean,
        Err(_) => ShutdownOutcome::TimedOut,
    }
}

pub struct ShutdownSignals {
    terminate: Signal,
    interrupt: Signal,
}

impl ShutdownSignals {
    pub fn install() -> std::io::Result<Self> {
        Ok(Self {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
        })
    }

    pub async fn wait(&mut self) {
        tokio::select! {
            _ = self.terminate.recv() => tracing::info!("SIGTERM reçu, arrêt en cours"),
            _ = self.interrupt.recv() => tracing::info!("SIGINT reçu, arrêt en cours"),
        }
    }
}
