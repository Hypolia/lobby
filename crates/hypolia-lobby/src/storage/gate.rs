use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::{OwnedRwLockReadGuard, RwLock};

use super::RepositoryError;

#[derive(Default)]
pub struct WriteGate {
    lock: Arc<RwLock<()>>,
    closed: AtomicBool,
}

impl WriteGate {
    pub fn try_enter(&self) -> Result<OwnedRwLockReadGuard<()>, RepositoryError> {
        let guard = Arc::clone(&self.lock)
            .try_read_owned()
            .map_err(|_| shutting_down())?;
        if self.closed.load(Ordering::SeqCst) {
            return Err(shutting_down());
        }
        Ok(guard)
    }

    pub async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        drop(self.lock.write().await);
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

fn shutting_down() -> RepositoryError {
    RepositoryError::Unavailable("arrêt du plugin en cours".to_owned())
}
