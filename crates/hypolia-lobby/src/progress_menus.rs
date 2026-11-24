use hypolia_progression::{Catalog, CatalogEntry};
use hypolia_protocol::{CosmeticId, CosmeticSlot, GameKind, GameStats, PlayerProfile};
use rspigot_plugin::{CustomInventory, Enchantment, ItemStack, Material};

use crate::auth::UserClaims;
use crate::gadgets::{AuraTrail, LobbyGadget};
use crate::menus::{back_item, close_item, fill_border, player_head};
use crate::profile::{ShopItemState, rank_label, shop_item_state, unlock_label};

pub const PROFILE_ROWS: u8 = 5;
pub const SHOP_ROWS: u8 = 5;

pub const PROFILE_SLOT_INFO: u16 = 13;
pub const PROFILE_STATS_SLOTS: [(u16, GameKind); 2] =
    [(21, GameKind::SheepWars), (23, GameKind::TowerWars)];
pub const PROFILE_SLOT_SETTINGS: u16 = 29;
pub const PROFILE_SLOT_FERRISKEY: u16 = 31;
pub const PROFILE_SLOT_SHOP: u16 = 33;
pub const PROFILE_SLOT_CLOSE: u16 = 40;

pub const SHOP_SLOT_COINS: u16 = 4;
pub const SHOP_SLOT_BACK: u16 = 40;
pub const SHOP_ITEM_SLOTS: [u16; 12] = [10, 11, 12, 13, 14, 15, 19, 20, 21, 22, 23, 24];
pub const SHOP_TAB_SLOTS: [(u16, CosmeticSlot); 4] = [
    (38, CosmeticSlot::LobbyGadget),
    (39, CosmeticSlot::LobbyTrail),
    (41, CosmeticSlot::SheepTrail),
    (42, CosmeticSlot::VictoryEffect),
];
pub const UNAVAILABLE_SLOT: u16 = 22;

const BAR_WIDTH: u64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMenuKind {
    Profile,
    Shop(CosmeticSlot),
}

impl ProgressMenuKind {
    pub const fn rows(self) -> u8 {
        match self {
            Self::Profile => PROFILE_ROWS,
            Self::Shop(_) => SHOP_ROWS,
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Profile => "§e§lHypolia §8• §fMon profil",
            Self::Shop(_) => "§a§lHypolia §8• §fBoutique",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileMenuAction {
    OpenSettings,
    FerrisKey,
    OpenShop,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShopMenuAction {
    Page(CosmeticSlot),
    Item(CosmeticId),
    BackToCollectibles,
}

pub struct ProfileView<'a> {
    pub name: &'a str,
    pub profile: Option<&'a PlayerProfile>,
    pub claims: Option<&'a UserClaims>,
}

pub fn decode_profile_click(slot: i32) -> Option<ProfileMenuAction> {
    match u16::try_from(slot).ok()? {
        PROFILE_SLOT_SETTINGS => Some(ProfileMenuAction::OpenSettings),
        PROFILE_SLOT_FERRISKEY => Some(ProfileMenuAction::FerrisKey),
        PROFILE_SLOT_SHOP => Some(ProfileMenuAction::OpenShop),
        PROFILE_SLOT_CLOSE => Some(ProfileMenuAction::Close),
        _ => None,
    }
}

pub fn decode_shop_click(slot: i32, page: CosmeticSlot) -> Option<ShopMenuAction> {
    let slot = u16::try_from(slot).ok()?;
    if slot == SHOP_SLOT_BACK {
        return Some(ShopMenuAction::BackToCollectibles);
    }
    if let Some((_, tab)) = SHOP_TAB_SLOTS.iter().find(|(s, _)| *s == slot) {
        return Some(ShopMenuAction::Page(*tab));
    }
    let index = SHOP_ITEM_SLOTS.iter().position(|s| *s == slot)?;
    Catalog::in_slot(page)
        .nth(index)
        .map(|entry| ShopMenuAction::Item(entry.id))
}

pub const fn slot_title(slot: CosmeticSlot) -> &'static str {
    match slot {
        CosmeticSlot::LobbyGadget => "§b§lGadgets",
        CosmeticSlot::LobbyTrail => "§d§lTraînées du lobby",
        CosmeticSlot::SheepTrail => "§f§lTraînées de mouton",
        CosmeticSlot::VictoryEffect => "§6§lEffets de victoire",
    }
}

const fn slot_material(slot: CosmeticSlot) -> Material {
    match slot {
        CosmeticSlot::LobbyGadget => Material::Feather,
        CosmeticSlot::LobbyTrail => Material::BlazePowder,
        CosmeticSlot::SheepTrail => Material::WhiteWool,
        CosmeticSlot::VictoryEffect => Material::FireworkRocket,
    }
}

pub fn cosmetic_title(id: CosmeticId) -> &'static str {
    if let Some(gadget) = LobbyGadget::from_cosmetic(id) {
        return gadget.title();
    }
    if let Some(trail) = AuraTrail::from_cosmetic(id) {
        return trail.label();
    }
    match id {
        CosmeticId::SheepTrailFlame => "§6Traînée de mouton : Flamme",
        CosmeticId::SheepTrailHearts => "§dTraînée de mouton : Cœurs",
        CosmeticId::SheepTrailNotes => "§aTraînée de mouton : Notes",
        CosmeticId::SheepTrailRainbow => "§bTraînée de mouton : Arc-en-ciel",
        CosmeticId::VictoryFireworks => "§cEffet de victoire : Feux d'artifice",
        CosmeticId::VictoryLightning => "§eEffet de victoire : Éclair",
        _ => "§7Cosmétique",
    }
}

pub fn cosmetic_material(id: CosmeticId) -> Material {
    if let Some(gadget) = LobbyGadget::from_cosmetic(id) {
        return gadget.material();
    }
    if let Some(trail) = AuraTrail::from_cosmetic(id) {
        return trail.material();
    }
    match id {
        CosmeticId::SheepTrailFlame => Material::BlazePowder,
        CosmeticId::SheepTrailHearts => Material::Poppy,
        CosmeticId::SheepTrailNotes => Material::NoteBlock,
        CosmeticId::SheepTrailRainbow => Material::PinkWool,
        CosmeticId::VictoryFireworks => Material::FireworkRocket,
        CosmeticId::VictoryLightning => Material::BlazeRod,
        _ => Material::Barrier,
    }
}

pub fn progress_bar(into: u64, needed: u64) -> String {
    let filled = into
        .saturating_mul(BAR_WIDTH)
        .checked_div(needed)
        .map_or(BAR_WIDTH, |filled| filled.min(BAR_WIDTH));
    let empty = BAR_WIDTH - filled;
    format!(
        "§a{}§7{}",
        "▌".repeat(filled as usize),
        "▌".repeat(empty as usize)
    )
}

pub const fn game_label(game: GameKind) -> &'static str {
    match game {
        GameKind::SheepWars => "SheepWars",
        GameKind::TowerWars => "TowerWars",
    }
}

pub fn shop_item(entry: &CatalogEntry, state: ShopItemState) -> ItemStack {
    let title = cosmetic_title(entry.id);
    let base = ItemStack::new(cosmetic_material(entry.id), 1);
    match state {
        ShopItemState::Equipped => base
            .name(format!("{title} §a§l[ÉQUIPÉ]"))
            .lore(["§a✔ Actuellement équipé"])
            .enchant(Enchantment::Unbreaking, 1),
        ShopItemState::Unlocked => base
            .name(format!("{title} §a§l[DÉBLOQUÉ]"))
            .lore(["§e▶ Cliquez pour équiper"]),
        ShopItemState::Buyable(price) => base.name(title).lore([
            format!("§7Prix : §6{} coins", price.get()),
            "§a▶ Cliquez pour acheter".to_owned(),
        ]),
        ShopItemState::Locked(unlock) => base
            .name(format!("{title} §c§l[VERROUILLÉ]"))
            .lore([format!("§c✖ {}", unlock_label(unlock))]),
    }
}

fn clear(inv: &CustomInventory, rows: u8) {
    for slot in 0..u16::from(rows) * 9 {
        inv.set(slot, None);
    }
}

fn populate_shop_unavailable(inv: &CustomInventory, kind: ProgressMenuKind, back: u16) {
    clear(inv, kind.rows());
    inv.set(
        UNAVAILABLE_SLOT,
        Some(
            ItemStack::new(Material::Barrier, 1)
                .name("§c§lBoutique indisponible")
                .lore([
                    "§7Le service de progression ne répond pas.",
                    "§7Réessayez dans quelques instants.",
                ]),
        ),
    );
    inv.set(back, Some(back_item("aux collectibles")));
}

pub fn populate_progress_menu(inv: &CustomInventory, kind: ProgressMenuKind, view: &ProfileView) {
    match kind {
        ProgressMenuKind::Profile => populate_profile_menu(inv, view),
        ProgressMenuKind::Shop(page) => populate_shop_menu(inv, page, view.profile),
    }
}

fn ferriskey_line(claims: Option<&UserClaims>) -> String {
    match claims {
        Some(user) => format!("§7FerrisKey : §a✔ {}", user.username),
        None => "§7FerrisKey : §cnon lié".to_owned(),
    }
}

fn info_item(view: &ProfileView) -> ItemStack {
    let mut lore = Vec::new();
    match view.profile {
        Some(profile) => {
            let network = &profile.network;
            lore.push(format!("§7Grade : §f{}", rank_label(profile.rank)));
            lore.push(format!("§7Level réseau : §f{}", network.level));
            lore.push(progress_bar(network.xp_into_level, network.xp_for_next));
            lore.push(format!(
                "§7XP : §f{}§7/§f{} §8(total {})",
                network.xp_into_level, network.xp_for_next, network.total_xp
            ));
            lore.push(format!("§6Coins : §f{}", profile.coins));
        }
        None => lore.push("§cProgression indisponible.".to_owned()),
    }
    lore.push(ferriskey_line(view.claims));
    player_head(view.name)
        .name(format!("§e§l{}", view.name))
        .lore(lore)
}

const fn game_material(game: GameKind) -> Material {
    match game {
        GameKind::SheepWars => Material::WhiteWool,
        GameKind::TowerWars => Material::Crossbow,
    }
}

fn win_rate(stats: &GameStats) -> u64 {
    (u64::from(stats.wins) * 100)
        .checked_div(u64::from(stats.played))
        .unwrap_or(0)
}

fn stats_item(game: GameKind, profile: Option<&PlayerProfile>) -> ItemStack {
    let base = ItemStack::new(game_material(game), 1);
    let Some(profile) = profile else {
        return base
            .name(format!("§7§l{}", game_label(game)))
            .lore(["§cStatistiques indisponibles."]);
    };
    let Some(stats) = profile.games.iter().find(|s| s.game == game) else {
        return base
            .name(format!("§f§l{} §7• Level §f1", game_label(game)))
            .lore(["§7Aucune partie jouée."]);
    };
    base.name(format!(
        "§f§l{} §7• Level §f{}",
        game_label(game),
        stats.progress.level
    ))
    .lore([
        progress_bar(stats.progress.xp_into_level, stats.progress.xp_for_next),
        String::new(),
        format!("§7Parties : §f{}", stats.played),
        format!("§aVictoires : §f{} §8({}%)", stats.wins, win_rate(stats)),
        format!("§cDéfaites : §f{}", stats.losses),
        format!("§eÉgalités : §f{}", stats.draws),
    ])
}

fn ferriskey_item(claims: Option<&UserClaims>) -> ItemStack {
    match claims {
        Some(user) => ItemStack::new(Material::Emerald, 1)
            .name("§a§lCompte FerrisKey")
            .lore([
                format!("§7Pseudo : §f{}", user.username),
                format!(
                    "§7E-mail : §f{}",
                    user.email.as_deref().unwrap_or("non renseigné")
                ),
                String::new(),
                "§c▶ Cliquez pour délier".to_owned(),
            ]),
        None => ItemStack::new(Material::AmethystShard, 1)
            .name("§e§lCompte FerrisKey")
            .lore([
                "§7Non lié. Donne accès aux grades",
                "§7et à la Capitainerie.",
                "",
                "§a▶ Cliquez pour lier",
            ]),
    }
}

pub fn populate_profile_menu(inv: &CustomInventory, view: &ProfileView) {
    clear(inv, PROFILE_ROWS);
    fill_border(
        inv,
        PROFILE_ROWS,
        Material::YellowStainedGlassPane,
        Material::GrayStainedGlassPane,
    );
    inv.set(PROFILE_SLOT_INFO, Some(info_item(view)));
    for (slot, game) in PROFILE_STATS_SLOTS {
        inv.set(slot, Some(stats_item(game, view.profile)));
    }
    inv.set(
        PROFILE_SLOT_SETTINGS,
        Some(
            ItemStack::new(Material::Comparator, 1)
                .name("§6§lParamètres")
                .lore([
                    "§7Visibilité, vol, vitesse, sons.",
                    "",
                    "§e▶ Cliquez pour ouvrir",
                ]),
        ),
    );
    inv.set(PROFILE_SLOT_FERRISKEY, Some(ferriskey_item(view.claims)));
    let coins_line = view.profile.map_or_else(
        || "§7Coins : §findisponible".to_owned(),
        |p| format!("§6Coins : §f{}", p.coins),
    );
    inv.set(
        PROFILE_SLOT_SHOP,
        Some(
            ItemStack::new(Material::GoldIngot, 1)
                .name("§a§lBoutique")
                .lore([
                    coins_line,
                    String::new(),
                    "§e▶ Cliquez pour ouvrir".to_owned(),
                ]),
        ),
    );
    inv.set(PROFILE_SLOT_CLOSE, Some(close_item()));
}

pub fn populate_shop_menu(
    inv: &CustomInventory,
    page: CosmeticSlot,
    profile: Option<&PlayerProfile>,
) {
    let kind = ProgressMenuKind::Shop(page);
    let Some(profile) = profile else {
        populate_shop_unavailable(inv, kind, SHOP_SLOT_BACK);
        return;
    };
    clear(inv, SHOP_ROWS);
    fill_border(
        inv,
        SHOP_ROWS,
        Material::LimeStainedGlassPane,
        Material::GrayStainedGlassPane,
    );
    inv.set(
        SHOP_SLOT_COINS,
        Some(
            ItemStack::new(Material::GoldIngot, 1)
                .name(format!("§6Coins §f{}", profile.coins))
                .lore([slot_title(page)]),
        ),
    );
    for (slot, entry) in SHOP_ITEM_SLOTS.iter().zip(Catalog::in_slot(page)) {
        inv.set(
            *slot,
            Some(shop_item(&entry, shop_item_state(profile, &entry))),
        );
    }
    for (slot, tab) in SHOP_TAB_SLOTS {
        let mut item = ItemStack::new(slot_material(tab), 1).name(slot_title(tab));
        if tab == page {
            item = item
                .lore(["§a✔ Page affichée"])
                .enchant(Enchantment::Unbreaking, 1);
        } else {
            item = item.lore(["§e▶ Cliquez pour afficher"]);
        }
        inv.set(slot, Some(item));
    }
    inv.set(SHOP_SLOT_BACK, Some(back_item("aux collectibles")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shop_items_decode_in_catalogue_order_per_page() {
        let first = SHOP_ITEM_SLOTS[0];
        assert_eq!(
            decode_shop_click(i32::from(first), CosmeticSlot::LobbyGadget),
            Some(ShopMenuAction::Item(CosmeticId::GadgetFeatherDash))
        );
        assert_eq!(
            decode_shop_click(i32::from(first), CosmeticSlot::VictoryEffect),
            Some(ShopMenuAction::Item(CosmeticId::VictoryFireworks))
        );
        assert_eq!(
            decode_shop_click(i32::from(SHOP_ITEM_SLOTS[5]), CosmeticSlot::LobbyGadget),
            Some(ShopMenuAction::Item(CosmeticId::GadgetEnderGrapple))
        );
        assert_eq!(
            decode_shop_click(i32::from(SHOP_ITEM_SLOTS[5]), CosmeticSlot::VictoryEffect),
            None
        );
    }

    #[test]
    fn shop_tabs_and_back_decode() {
        for (slot, tab) in SHOP_TAB_SLOTS {
            assert_eq!(
                decode_shop_click(i32::from(slot), CosmeticSlot::LobbyGadget),
                Some(ShopMenuAction::Page(tab))
            );
        }
        assert_eq!(
            decode_shop_click(i32::from(SHOP_SLOT_BACK), CosmeticSlot::SheepTrail),
            Some(ShopMenuAction::BackToCollectibles)
        );
        assert_eq!(decode_shop_click(-1, CosmeticSlot::SheepTrail), None);
        assert_eq!(decode_shop_click(0, CosmeticSlot::SheepTrail), None);
    }

    #[test]
    fn profile_slots_decode() {
        assert_eq!(
            decode_profile_click(i32::from(PROFILE_SLOT_SETTINGS)),
            Some(ProfileMenuAction::OpenSettings)
        );
        assert_eq!(
            decode_profile_click(i32::from(PROFILE_SLOT_FERRISKEY)),
            Some(ProfileMenuAction::FerrisKey)
        );
        assert_eq!(
            decode_profile_click(i32::from(PROFILE_SLOT_SHOP)),
            Some(ProfileMenuAction::OpenShop)
        );
        assert_eq!(
            decode_profile_click(i32::from(PROFILE_SLOT_CLOSE)),
            Some(ProfileMenuAction::Close)
        );
        for (slot, _) in PROFILE_STATS_SLOTS {
            assert_eq!(decode_profile_click(i32::from(slot)), None);
        }
        assert_eq!(decode_profile_click(0), None);
    }

    #[test]
    fn every_page_fits_in_the_item_slots() {
        for (_, page) in SHOP_TAB_SLOTS {
            assert!(Catalog::in_slot(page).count() <= SHOP_ITEM_SLOTS.len());
        }
    }

    #[test]
    fn every_catalogue_cosmetic_has_a_title() {
        for id in Catalog::ALL {
            assert_ne!(cosmetic_title(id), "§7Cosmétique", "{id:?}");
            assert_ne!(cosmetic_material(id), Material::Barrier, "{id:?}");
        }
    }

    #[test]
    fn progress_bar_fills_proportionally_and_never_overflows() {
        assert_eq!(progress_bar(0, 100), format!("§a§7{}", "▌".repeat(20)));
        assert_eq!(
            progress_bar(50, 100),
            format!("§a{}§7{}", "▌".repeat(10), "▌".repeat(10))
        );
        assert_eq!(progress_bar(500, 100), format!("§a{}§7", "▌".repeat(20)));
        assert_eq!(progress_bar(1, 0), format!("§a{}§7", "▌".repeat(20)));
    }
}
