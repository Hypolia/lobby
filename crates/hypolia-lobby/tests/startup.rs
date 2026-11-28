use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use hypolia_lobby::config::{LobbyLayout, WaypointPose};
use hypolia_lobby::storage::{Backoff, LayoutRepository, RepositoryError, load_or_seed};

struct Flaky {
    failures: usize,
    calls: AtomicUsize,
    saved: std::sync::Mutex<Option<LobbyLayout>>,
    stored: Option<LobbyLayout>,
}

impl Flaky {
    fn new(failures: usize, stored: Option<LobbyLayout>) -> Self {
        Self {
            failures,
            calls: AtomicUsize::new(0),
            saved: std::sync::Mutex::new(None),
            stored,
        }
    }
}

impl LayoutRepository for Flaky {
    async fn load(&self) -> Result<Option<LobbyLayout>, RepositoryError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call < self.failures {
            return Err(RepositoryError::Unavailable("base injoignable".to_owned()));
        }
        Ok(self.stored.clone())
    }

    async fn save(&self, layout: &LobbyLayout) -> Result<(), RepositoryError> {
        *self.saved.lock().expect("lock") = Some(layout.clone());
        Ok(())
    }
}

fn fast(budget: Option<Duration>) -> Backoff {
    Backoff {
        initial: Duration::from_millis(5),
        max: Duration::from_millis(20),
        budget,
    }
}

fn admin_layout() -> LobbyLayout {
    LobbyLayout {
        spawn: WaypointPose::new(99.5, 10.0, 99.5, 0.0, 0.0),
        ..LobbyLayout::default()
    }
}

#[tokio::test]
async fn unreachable_database_is_retried_and_the_stored_layout_wins_over_the_seed() {
    let repo = Flaky::new(3, Some(admin_layout()));
    let layout = load_or_seed(&repo, &LobbyLayout::default(), fast(None))
        .await
        .expect("eventually loads");
    assert_eq!(layout, admin_layout());
    assert_eq!(repo.calls.load(Ordering::SeqCst), 4);
    assert!(repo.saved.lock().expect("lock").is_none());
}

#[tokio::test]
async fn startup_never_falls_back_to_an_empty_layout() {
    let repo = Flaky::new(usize::MAX, Some(admin_layout()));
    let err = load_or_seed(
        &repo,
        &LobbyLayout::default(),
        fast(Some(Duration::from_millis(100))),
    )
    .await
    .expect_err("never starts empty");
    assert!(matches!(err, RepositoryError::Unavailable(_)));
    assert!(repo.saved.lock().expect("lock").is_none());
}

#[tokio::test]
async fn empty_store_is_seeded_with_the_bundled_layout() {
    let repo = Flaky::new(0, None);
    let seed = LobbyLayout::default();
    let layout = load_or_seed(&repo, &seed, fast(None))
        .await
        .expect("seeded");
    assert_eq!(layout, seed);
    assert_eq!(*repo.saved.lock().expect("lock"), Some(seed));
}

fn legacy_layout_with_uhc_pedestal() -> LobbyLayout {
    let mut layout = admin_layout();
    let slot = layout
        .pedestals
        .iter_mut()
        .find(|p| p.id == "towerwars")
        .expect("default has towerwars");
    slot.id = "uhc".to_owned();
    slot.game_id = "uhc".to_owned();
    slot.active = false;
    slot.pose = WaypointPose::new(7.5, 30.0, -70.5, 90.0, 0.0);
    layout
}

#[tokio::test]
async fn stored_layout_without_towerwars_gets_it_in_place_of_the_inactive_uhc_slot() {
    let repo = Flaky::new(0, Some(legacy_layout_with_uhc_pedestal()));
    let layout = load_or_seed(&repo, &LobbyLayout::default(), fast(None))
        .await
        .expect("loads");
    let tower = layout
        .pedestals
        .iter()
        .find(|p| p.id == "towerwars")
        .expect("towerwars pedestal added");
    assert!(tower.active);
    assert_eq!(tower.game_id, "towerwars");
    assert_eq!(tower.pose, WaypointPose::new(7.5, 30.0, -70.5, 90.0, 0.0));
    assert!(layout.pedestals.iter().all(|p| p.id != "uhc"));
    assert_eq!(layout.spawn, admin_layout().spawn);
    assert_eq!(*repo.saved.lock().expect("lock"), Some(layout));
}

#[tokio::test]
async fn stored_layout_without_a_leaderboard_position_gets_the_bundled_one() {
    let mut stored = admin_layout();
    stored.leaderboard = None;
    let repo = Flaky::new(0, Some(stored));
    let seed = LobbyLayout::default();
    assert!(seed.leaderboard.is_some());
    let layout = load_or_seed(&repo, &seed, fast(None)).await.expect("loads");
    assert_eq!(layout.leaderboard, seed.leaderboard);
    assert_eq!(*repo.saved.lock().expect("lock"), Some(layout));
}

#[tokio::test]
async fn a_leaderboard_position_set_by_an_admin_is_never_overwritten() {
    let mine = WaypointPose::new(1.5, 40.0, 2.5, 0.0, 0.0);
    let mut stored = admin_layout();
    stored.leaderboard = Some(mine);
    let repo = Flaky::new(0, Some(stored));
    let layout = load_or_seed(&repo, &LobbyLayout::default(), fast(None))
        .await
        .expect("loads");
    assert_eq!(layout.leaderboard, Some(mine));
    assert!(repo.saved.lock().expect("lock").is_none());
}

#[tokio::test]
async fn a_seed_without_a_position_leaves_the_stored_layout_alone() {
    let mut stored = admin_layout();
    stored.leaderboard = None;
    let seed = LobbyLayout {
        leaderboard: None,
        ..LobbyLayout::default()
    };
    let repo = Flaky::new(0, Some(stored));
    let layout = load_or_seed(&repo, &seed, fast(None)).await.expect("loads");
    assert_eq!(layout.leaderboard, None);
    assert!(repo.saved.lock().expect("lock").is_none());
}
