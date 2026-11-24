use std::collections::HashMap;
use std::sync::RwLock;

use hypolia_progression::{Catalog, CatalogEntry, Coins, Unlock};
use hypolia_protocol::{CosmeticId, CosmeticSlot, PlayerProfile, PlayerRank};
use rspigot_plugin::Uuid;

use crate::auth::Rank;
use crate::gadgets::{AuraTrail, LobbyGadget};
use crate::orchestrator::ProgressionClientError;

pub const OFFLINE_USABLE: [CosmeticId; 2] = [
    CosmeticId::GadgetFeatherDash,
    CosmeticId::GadgetFireworkRocket,
];

pub const fn to_player_rank(rank: Rank) -> PlayerRank {
    match rank {
        Rank::Admin => PlayerRank::Admin,
        Rank::Mod => PlayerRank::Mod,
        Rank::Dev => PlayerRank::Dev,
        Rank::Vip => PlayerRank::Vip,
        Rank::Member => PlayerRank::Member,
        Rank::Guest => PlayerRank::Guest,
    }
}

pub const fn can_grant_coins(rank: Rank) -> bool {
    matches!(rank, Rank::Admin)
}

pub fn level_prefix(level: Option<u32>) -> String {
    match level {
        Some(level) => format!("§7[{level}✫] "),
        None => String::new(),
    }
}

pub const fn rank_label(rank: PlayerRank) -> &'static str {
    match rank {
        PlayerRank::Admin => "Admin",
        PlayerRank::Mod => "Mod",
        PlayerRank::Dev => "Dev",
        PlayerRank::Vip => "VIP",
        PlayerRank::Member => "Membre",
        PlayerRank::Guest => "Invité",
    }
}

pub fn unlock_label(unlock: Unlock) -> String {
    match unlock {
        Unlock::Free => "Gratuit".to_owned(),
        Unlock::Price(price) => format!("{} coins", price.get()),
        Unlock::Rank(rank) => format!("Grade {} requis", rank_label(rank)),
        Unlock::NetworkLevel(level) => format!("Level réseau {} requis", level.get()),
    }
}

pub fn requirement_message(unlock: Unlock) -> String {
    match unlock {
        Unlock::Free => "§cCe cosmétique n'est pas débloqué.".to_owned(),
        Unlock::Price(price) => format!(
            "§cAchète-le d'abord dans la boutique ({} coins).",
            price.get()
        ),
        Unlock::Rank(rank) => format!("§cGrade {} requis.", rank_label(rank)),
        Unlock::NetworkLevel(level) => format!("§cLevel réseau {} requis.", level.get()),
    }
}

pub fn locked_cosmetic_message(cosmetic: CosmeticId) -> String {
    Catalog::entry(cosmetic).map_or_else(
        || "§cCosmétique inconnu.".to_owned(),
        |entry| requirement_message(entry.unlock),
    )
}

pub fn error_message(err: &ProgressionClientError) -> String {
    match err {
        ProgressionClientError::InsufficientCoins { missing } => {
            format!("§cIl te manque {missing} coins.")
        }
        ProgressionClientError::RankRequired(rank) => {
            format!("§cGrade {} requis.", rank_label(*rank))
        }
        ProgressionClientError::LevelRequired(level) => {
            format!("§cLevel réseau {level} requis.")
        }
        ProgressionClientError::AlreadyUnlocked => "§eTu possèdes déjà ce cosmétique.".to_owned(),
        ProgressionClientError::UnknownCosmetic => "§cCosmétique inconnu.".to_owned(),
        ProgressionClientError::NotUnlocked { requires } => requirement_message(*requires),
        ProgressionClientError::WrongSlot => "§cMauvais emplacement.".to_owned(),
        ProgressionClientError::Transport(_)
        | ProgressionClientError::Unavailable
        | ProgressionClientError::HttpStatus(_) => "§cBoutique indisponible.".to_owned(),
    }
}

pub fn usable(profile: Option<&PlayerProfile>, cosmetic: CosmeticId) -> bool {
    match profile {
        Some(profile) => profile.unlocked.contains(&cosmetic),
        None => OFFLINE_USABLE.contains(&cosmetic),
    }
}

pub fn gadget_usable(profile: Option<&PlayerProfile>, gadget: LobbyGadget) -> bool {
    usable(profile, gadget.cosmetic())
}

pub fn trail_usable(profile: Option<&PlayerProfile>, trail: AuraTrail) -> bool {
    trail.cosmetic().is_none_or(|id| usable(profile, id))
}

pub fn next_usable_trail(current: AuraTrail, profile: Option<&PlayerProfile>) -> AuraTrail {
    let mut candidate = current.next();
    while candidate != current {
        if trail_usable(profile, candidate) {
            return candidate;
        }
        candidate = candidate.next();
    }
    current
}

pub fn equipped_gadget(profile: &PlayerProfile) -> Option<LobbyGadget> {
    profile
        .equipped
        .get(&CosmeticSlot::LobbyGadget)
        .and_then(|id| LobbyGadget::from_cosmetic(*id))
}

pub fn equipped_trail(profile: &PlayerProfile) -> Option<AuraTrail> {
    profile
        .equipped
        .get(&CosmeticSlot::LobbyTrail)
        .and_then(|id| AuraTrail::from_cosmetic(*id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShopItemState {
    Equipped,
    Unlocked,
    Buyable(Coins),
    Locked(Unlock),
}

pub fn shop_item_state(profile: &PlayerProfile, entry: &CatalogEntry) -> ShopItemState {
    let owned = profile.unlocked.contains(&entry.id) || entry.unlock == Unlock::Free;
    if owned {
        return if profile.equipped.get(&entry.slot) == Some(&entry.id) {
            ShopItemState::Equipped
        } else {
            ShopItemState::Unlocked
        };
    }
    match entry.unlock {
        Unlock::Price(price) => ShopItemState::Buyable(price),
        other => ShopItemState::Locked(other),
    }
}

#[derive(Default)]
pub struct ProfileCache {
    profiles: RwLock<HashMap<Uuid, PlayerProfile>>,
}

impl ProfileCache {
    pub fn store(&self, uuid: Uuid, profile: PlayerProfile) {
        self.profiles
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(uuid, profile);
    }

    pub fn snapshot(&self, uuid: Uuid) -> Option<PlayerProfile> {
        self.with(uuid, |p| p.cloned())
    }

    pub fn remove(&self, uuid: Uuid) {
        self.profiles
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&uuid);
    }

    pub fn with<R>(&self, uuid: Uuid, f: impl FnOnce(Option<&PlayerProfile>) -> R) -> R {
        f(self
            .profiles
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&uuid))
    }

    pub fn level(&self, uuid: Uuid) -> Option<u32> {
        self.with(uuid, |p| p.map(|p| p.network.level))
    }

    pub fn level_percent(&self, uuid: Uuid) -> Option<u32> {
        self.with(uuid, |p| {
            p.map(|p| {
                let next = p.network.xp_for_next.max(1);
                u32::try_from(p.network.xp_into_level.min(next) * 100 / next).unwrap_or(100)
            })
        })
    }

    pub fn coins(&self, uuid: Uuid) -> Option<u64> {
        self.with(uuid, |p| p.map(|p| p.coins))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use hypolia_progression::{Catalog, Level};
    use hypolia_protocol::LevelProgress;

    use super::*;

    fn profile(unlocked: &[CosmeticId], equipped: &[(CosmeticSlot, CosmeticId)]) -> PlayerProfile {
        PlayerProfile {
            uuid: Uuid::nil(),
            name: "A".to_owned(),
            rank: PlayerRank::Member,
            coins: 600,
            network: LevelProgress {
                level: 12,
                xp_into_level: 0,
                xp_for_next: 1000,
                total_xp: 0,
            },
            games: Vec::new(),
            unlocked: unlocked.to_vec(),
            equipped: equipped.iter().copied().collect::<BTreeMap<_, _>>(),
        }
    }

    fn entry(id: CosmeticId) -> CatalogEntry {
        Catalog::entry(id).expect("catalog entry")
    }

    #[test]
    fn lobby_ranks_map_one_to_one_onto_player_ranks() {
        let pairs = [
            (Rank::Admin, PlayerRank::Admin),
            (Rank::Mod, PlayerRank::Mod),
            (Rank::Dev, PlayerRank::Dev),
            (Rank::Vip, PlayerRank::Vip),
            (Rank::Member, PlayerRank::Member),
            (Rank::Guest, PlayerRank::Guest),
        ];
        for (lobby, wire) in pairs {
            assert_eq!(to_player_rank(lobby), wire);
        }
    }

    #[test]
    fn level_prefix_is_empty_until_a_profile_is_known() {
        assert_eq!(level_prefix(None), "");
        assert_eq!(level_prefix(Some(12)), "§7[12✫] ");
    }

    #[test]
    fn only_admins_can_grant_coins() {
        for rank in Rank::ALL {
            assert_eq!(can_grant_coins(rank), rank == Rank::Admin, "{rank:?}");
        }
    }

    #[test]
    fn without_a_profile_only_the_two_free_gadgets_are_usable() {
        for gadget in LobbyGadget::ALL {
            let free = matches!(
                gadget,
                LobbyGadget::FeatherDash | LobbyGadget::FireworkRocket
            );
            assert_eq!(gadget_usable(None, gadget), free, "{gadget:?}");
        }
        assert!(trail_usable(None, AuraTrail::None));
        assert!(!trail_usable(None, AuraTrail::Flame));
    }

    #[test]
    fn with_a_profile_only_unlocked_cosmetics_are_usable() {
        let p = profile(&[CosmeticId::GadgetWindCharge, CosmeticId::TrailHeart], &[]);
        assert!(gadget_usable(Some(&p), LobbyGadget::WindCharge));
        assert!(!gadget_usable(Some(&p), LobbyGadget::FeatherDash));
        assert!(trail_usable(Some(&p), AuraTrail::Heart));
        assert!(!trail_usable(Some(&p), AuraTrail::Emerald));
    }

    #[test]
    fn trail_cycling_skips_locked_trails() {
        let p = profile(&[CosmeticId::TrailHeart], &[]);
        assert_eq!(
            next_usable_trail(AuraTrail::None, Some(&p)),
            AuraTrail::Heart
        );
        assert_eq!(
            next_usable_trail(AuraTrail::Heart, Some(&p)),
            AuraTrail::None
        );
        assert_eq!(next_usable_trail(AuraTrail::None, None), AuraTrail::None);
    }

    #[test]
    fn equipped_lobby_cosmetics_are_read_from_the_profile() {
        let p = profile(
            &[],
            &[
                (CosmeticSlot::LobbyGadget, CosmeticId::GadgetSheepCannon),
                (CosmeticSlot::LobbyTrail, CosmeticId::TrailMagic),
            ],
        );
        assert_eq!(equipped_gadget(&p), Some(LobbyGadget::SheepCannon));
        assert_eq!(equipped_trail(&p), Some(AuraTrail::Magic));
        assert_eq!(equipped_gadget(&profile(&[], &[])), None);
    }

    #[test]
    fn shop_state_covers_equipped_unlocked_buyable_and_locked() {
        let p = profile(
            &[
                CosmeticId::GadgetFeatherDash,
                CosmeticId::GadgetFireworkRocket,
                CosmeticId::TrailHeart,
            ],
            &[(CosmeticSlot::LobbyGadget, CosmeticId::GadgetFeatherDash)],
        );
        assert_eq!(
            shop_item_state(&p, &entry(CosmeticId::GadgetFeatherDash)),
            ShopItemState::Equipped
        );
        assert_eq!(
            shop_item_state(&p, &entry(CosmeticId::GadgetFireworkRocket)),
            ShopItemState::Unlocked
        );
        assert_eq!(
            shop_item_state(&p, &entry(CosmeticId::TrailHeart)),
            ShopItemState::Unlocked
        );
        assert_eq!(
            shop_item_state(&p, &entry(CosmeticId::GadgetWindCharge)),
            ShopItemState::Buyable(Coins(500))
        );
        assert_eq!(
            shop_item_state(&p, &entry(CosmeticId::TrailEmerald)),
            ShopItemState::Locked(Unlock::Rank(PlayerRank::Vip))
        );
        assert_eq!(
            shop_item_state(&p, &entry(CosmeticId::TrailMagic)),
            ShopItemState::Locked(Unlock::NetworkLevel(Level::new(10).expect("non zero")))
        );
    }

    #[test]
    fn errors_become_clear_french_messages() {
        assert_eq!(
            error_message(&ProgressionClientError::InsufficientCoins { missing: 100 }),
            "§cIl te manque 100 coins."
        );
        assert_eq!(
            error_message(&ProgressionClientError::RankRequired(PlayerRank::Vip)),
            "§cGrade VIP requis."
        );
        assert_eq!(
            error_message(&ProgressionClientError::LevelRequired(10)),
            "§cLevel réseau 10 requis."
        );
        assert_eq!(
            error_message(&ProgressionClientError::NotUnlocked {
                requires: Unlock::Price(Coins(500))
            }),
            "§cAchète-le d'abord dans la boutique (500 coins)."
        );
        assert_eq!(
            error_message(&ProgressionClientError::Unavailable),
            "§cBoutique indisponible."
        );
        assert_eq!(
            error_message(&ProgressionClientError::AlreadyUnlocked),
            "§eTu possèdes déjà ce cosmétique."
        );
    }

    #[test]
    fn locked_cosmetics_report_their_catalogue_requirement() {
        assert_eq!(
            locked_cosmetic_message(CosmeticId::GadgetEnderGrapple),
            "§cGrade VIP requis."
        );
        assert_eq!(
            locked_cosmetic_message(CosmeticId::TrailMagic),
            "§cLevel réseau 10 requis."
        );
        assert_eq!(
            locked_cosmetic_message(CosmeticId::GadgetWindCharge),
            "§cAchète-le d'abord dans la boutique (500 coins)."
        );
        assert_eq!(
            locked_cosmetic_message(CosmeticId::Unknown),
            "§cCosmétique inconnu."
        );
    }

    #[test]
    fn cache_drops_profiles_and_exposes_level_and_coins() {
        let cache = ProfileCache::default();
        let uuid = Uuid::new_v4();
        assert_eq!(cache.level(uuid), None);
        cache.store(uuid, profile(&[], &[]));
        assert_eq!(cache.level(uuid), Some(12));
        assert_eq!(cache.coins(uuid), Some(600));
        cache.remove(uuid);
        assert_eq!(cache.coins(uuid), None);
    }
}
