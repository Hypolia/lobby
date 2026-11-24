use std::future::Future;
use std::time::Duration;

use tokio::time::Instant;

use super::RepositoryError;

#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
    pub budget: Option<Duration>,
}

impl Backoff {
    pub const fn startup() -> Self {
        Self {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(10),
            budget: None,
        }
    }

    pub const fn migration() -> Self {
        Self {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(10),
            budget: Some(Duration::from_secs(120)),
        }
    }
}

pub async fn retry_unavailable<T, F, Fut>(
    policy: Backoff,
    what: &str,
    mut op: F,
) -> Result<T, RepositoryError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, RepositoryError>>,
{
    let started = Instant::now();
    let mut delay = policy.initial;
    loop {
        let err = match op().await {
            Ok(value) => return Ok(value),
            Err(err @ RepositoryError::Unavailable(_)) => err,
            Err(other) => return Err(other),
        };
        if let Some(budget) = policy.budget
            && started.elapsed() + delay > budget
        {
            return Err(err);
        }
        tracing::warn!(
            "{what} impossible, nouvelle tentative dans {} ms : {err}",
            delay.as_millis()
        );
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(policy.max);
    }
}
