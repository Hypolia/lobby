use std::collections::HashMap;
use std::sync::RwLock;

use rspigot_plugin::{Player, Uuid};

use crate::gadgets::{AuraTrail, LobbyGadget};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedTier {
    Normal,
    Fast,
    Sprint,
}

impl SpeedTier {
    pub const fn walk_speed(self) -> f32 {
        match self {
            Self::Normal => 0.2,
            Self::Fast => 0.28,
            Self::Sprint => 0.36,
        }
    }

    pub const fn fly_speed(self) -> f32 {
        match self {
            Self::Normal => 0.10,
            Self::Fast => 0.14,
            Self::Sprint => 0.18,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "§aNormale (1.0×)",
            Self::Fast => "§bRapide (1.4×)",
            Self::Sprint => "§dSprint (1.8×)",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Normal => Self::Fast,
            Self::Fast => Self::Sprint,
            Self::Sprint => Self::Normal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerPrefs {
    pub players_visible: bool,
    pub flight_enabled: bool,
    pub speed_tier: SpeedTier,
    pub chat_sounds: bool,
    pub build_mode: bool,
    pub equipped_gadget: Option<LobbyGadget>,
    pub aura_trail: AuraTrail,
}

impl Default for PlayerPrefs {
    fn default() -> Self {
        Self {
            players_visible: true,
            flight_enabled: false,
            speed_tier: SpeedTier::Normal,
            chat_sounds: true,
            build_mode: false,
            equipped_gadget: Some(LobbyGadget::FeatherDash),
            aura_trail: AuraTrail::None,
        }
    }
}

impl PlayerPrefs {
    pub fn apply_movement(&self, player: &Player) {
        player.abilities.set_allow_flight(self.flight_enabled);
        if !self.flight_enabled {
            player.abilities.set_flying(false);
        }
        player.abilities.set_fly_speed(self.speed_tier.fly_speed());
        player
            .abilities
            .set_walk_speed(self.speed_tier.walk_speed());
    }
}

#[derive(Default)]
pub struct PrefsStore {
    by_player: RwLock<HashMap<Uuid, PlayerPrefs>>,
}

impl PrefsStore {
    pub fn get(&self, uuid: Uuid) -> PlayerPrefs {
        self.by_player
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&uuid)
            .copied()
            .unwrap_or_default()
    }

    pub fn update<F>(&self, uuid: Uuid, mutate: F) -> PlayerPrefs
    where
        F: FnOnce(&mut PlayerPrefs),
    {
        let mut guard = self.by_player.write().unwrap_or_else(|e| e.into_inner());
        let entry = guard.entry(uuid).or_default();
        mutate(entry);
        *entry
    }

    pub fn viewers_hiding_others(&self) -> Vec<Uuid> {
        self.by_player
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter_map(|(uuid, prefs)| (!prefs.players_visible).then_some(*uuid))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_tiers_cycle_in_order() {
        assert_eq!(SpeedTier::Normal.next(), SpeedTier::Fast);
        assert_eq!(SpeedTier::Fast.next(), SpeedTier::Sprint);
        assert_eq!(SpeedTier::Sprint.next(), SpeedTier::Normal);
    }

    #[test]
    fn prefs_store_tracks_visibility_and_mutations() {
        let store = PrefsStore::default();
        let id = Uuid::new_v4();
        assert!(store.get(id).players_visible);
        let updated = store.update(id, |p| p.players_visible = false);
        assert!(!updated.players_visible);
        assert_eq!(store.viewers_hiding_others(), vec![id]);
    }
}
