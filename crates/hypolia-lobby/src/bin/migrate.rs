use std::process::ExitCode;

use hypolia_lobby::storage::{
    Backoff, DATABASE_URL_ENV, connect_pool, database_url, migrate, retry_unavailable,
};

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let Some(url) = database_url() else {
        tracing::error!("{DATABASE_URL_ENV} n'est pas défini");
        return ExitCode::FAILURE;
    };

    let result = retry_unavailable(Backoff::migration(), "connexion Postgres", || {
        connect_pool(&url)
    })
    .await;
    let pool = match result {
        Ok(pool) => pool,
        Err(err) => {
            tracing::error!("Base injoignable : {err}");
            return ExitCode::FAILURE;
        }
    };

    match migrate(&pool).await {
        Ok(()) => {
            tracing::info!("Migrations du lobby appliquées");
            ExitCode::SUCCESS
        }
        Err(err) => {
            tracing::error!("Migrations du lobby en échec : {err}");
            ExitCode::FAILURE
        }
    }
}
