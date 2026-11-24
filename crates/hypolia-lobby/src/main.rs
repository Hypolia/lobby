use std::process::ExitCode;
use std::sync::Arc;

use hypolia_lobby::HypoliaLobby;
use hypolia_lobby::auth::AuthManager;
use hypolia_lobby::config::{EnvConfig, LayoutStore};
use hypolia_lobby::heartbeat::HeartbeatReporter;
use hypolia_lobby::lifecycle::{SHUTDOWN_BUDGET, ShutdownSignals, graceful_shutdown};
use hypolia_lobby::storage::{
    Backends, Backoff, RepositoryError, WriteGate, bundled_layout, database_url,
};

async fn open_storage(
    config: &EnvConfig,
    gate: &Arc<WriteGate>,
) -> Result<(LayoutStore, AuthManager), RepositoryError> {
    let backends = Backends::open(config, Backoff::startup()).await?;
    let seed = bundled_layout(&config.layout_path)?;
    let layout =
        LayoutStore::open(backends.layout, &seed, Arc::clone(gate), Backoff::startup()).await?;
    let auth = AuthManager::with_backends(
        config.clone(),
        backends.lobby_sessions,
        backends.capitainerie_sessions,
        Arc::clone(gate),
    );
    Ok((layout, auth))
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    if let Err(err) = dotenvy::dotenv()
        && !err.not_found()
    {
        tracing::warn!("Impossible de charger le fichier .env : {err}");
    }

    let mut signals = match ShutdownSignals::install() {
        Ok(signals) => signals,
        Err(err) => {
            tracing::error!("Impossible d'écouter les signaux d'arrêt : {err}");
            return ExitCode::FAILURE;
        }
    };

    let config = EnvConfig::from_env();
    let gate = Arc::new(WriteGate::default());

    let opened = tokio::select! {
        opened = open_storage(&config, &gate) => opened,
        () = signals.wait() => return ExitCode::SUCCESS,
    };
    let (layout, auth) = match opened {
        Ok(opened) => opened,
        Err(err) => {
            tracing::error!("Impossible d'initialiser le stockage du lobby : {err}");
            return ExitCode::from(2);
        }
    };

    tracing::info!(
        "Démarrage de hypolia-lobby (layout={}, stockage={})",
        config.layout_path.display(),
        if database_url().is_some() {
            "postgres"
        } else {
            "fichiers"
        }
    );

    let heartbeat = HeartbeatReporter::from_env(&config).map(Arc::new);
    let plugin = HypoliaLobby::new(config, layout, auth, heartbeat.clone());

    tokio::select! {
        result = rspigot_plugin::run(plugin) => {
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    tracing::error!("Le plugin s'est arrêté : {err}");
                    ExitCode::FAILURE
                }
            }
        }
        () = signals.wait() => {
            let outcome = graceful_shutdown(&gate, heartbeat.as_deref(), SHUTDOWN_BUDGET).await;
            tracing::info!("Arrêt terminé ({outcome:?})");
            ExitCode::SUCCESS
        }
    }
}
