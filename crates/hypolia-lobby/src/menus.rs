use std::collections::HashMap;
use std::sync::RwLock;

use rspigot_plugin::{
    CustomInventory, Enchantment, Error, InventoryRef, InventorySlot, ItemStack, Material, Player,
    Profile, Server, SlotItem, Uuid,
};

use crate::auth::UserClaims;
use crate::gadgets::LobbyGadget;
use crate::orchestrator::QueueCounts;
use crate::prefs::PlayerPrefs;
use crate::profile::{gadget_usable, unlock_label};
use crate::progress_menus::{ProfileView, ProgressMenuKind, populate_progress_menu};
use crate::servers_menu::{SERVERS_ROWS, populate_servers_menu, sorted_server_ids};
use hypolia_progression::Catalog;
use hypolia_protocol::{FleetStatusSnapshot, PlayerProfile};

pub const HOTBAR_MENU_SLOT: u8 = 0;
pub const HOTBAR_PROFILE_SLOT: u8 = 1;
pub const HOTBAR_COLLECTIBLES_SLOT: u8 = 4;
pub const HOTBAR_EQUIPPED_GADGET_SLOT: u8 = 5;
pub const HOTBAR_VISIBILITY_SLOT: u8 = 8;
const HOTBAR_EMPTY_SLOTS: [u8; 4] = [2, 3, 6, 7];

pub const GAMES_ROWS: u8 = 5;
pub const OPTIONS_ROWS: u8 = 4;
pub const GADGETS_ROWS: u8 = 4;
pub const TOWERWARS_SIZE_ROWS: u8 = 1;

pub const SLOT_SHEEPWARS: u16 = 12;
pub const SLOT_TOWERWARS: u16 = 14;
pub const SLOT_TP_SPAWN: u16 = 29;
pub const SLOT_TP_PORTAL: u16 = 30;
pub const SLOT_TP_CAPITAINERIE: u16 = 32;
pub const SLOT_TP_CASTLE: u16 = 33;
pub const SLOT_GAMES_CLOSE: u16 = 40;

pub const OPT_SLOT_VISIBILITY: u16 = 10;
pub const OPT_SLOT_FLIGHT: u16 = 12;
pub const OPT_SLOT_SPEED: u16 = 14;
pub const OPT_SLOT_CHAT_SOUNDS: u16 = 16;
pub const OPT_SLOT_BACK: u16 = 31;

pub const GAD_SLOT_FEATHER: u16 = 10;
pub const GAD_SLOT_ROCKET: u16 = 11;
pub const GAD_SLOT_SHEEP: u16 = 12;
pub const GAD_SLOT_VORTEX: u16 = 13;
pub const GAD_SLOT_WIND: u16 = 14;
pub const GAD_SLOT_ENDER: u16 = 15;
pub const GAD_SLOT_UNEQUIP: u16 = 16;
pub const GAD_SLOT_TRAIL: u16 = 21;
pub const GAD_SLOT_SHOP: u16 = 23;
pub const GAD_SLOT_CLOSE: u16 = 31;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GamesMenuAction {
    JoinSheepWars,
    JoinTowerWars,
    TeleportPoi(&'static str),
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionsMenuAction {
    ToggleVisibility,
    ToggleFlight,
    CycleSpeed,
    ToggleChatSounds,
    BackToProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GadgetsMenuAction {
    Equip(LobbyGadget),
    UnequipGadget,
    CycleTrail,
    OpenShop,
    Close,
}

pub fn decode_games_click(slot: i32) -> Option<GamesMenuAction> {
    let slot = u16::try_from(slot).ok()?;
    match slot {
        SLOT_SHEEPWARS => Some(GamesMenuAction::JoinSheepWars),
        SLOT_TOWERWARS => Some(GamesMenuAction::JoinTowerWars),
        SLOT_TP_SPAWN => Some(GamesMenuAction::TeleportPoi("spawn")),
        SLOT_TP_PORTAL => Some(GamesMenuAction::TeleportPoi("portal")),
        SLOT_TP_CAPITAINERIE => Some(GamesMenuAction::TeleportPoi("capitainerie")),
        SLOT_TP_CASTLE => Some(GamesMenuAction::TeleportPoi("castle")),
        SLOT_GAMES_CLOSE => Some(GamesMenuAction::Close),
        _ => None,
    }
}

pub struct TowerWarsSizeOption {
    pub slot: u16,
    pub size: u8,
    pub name: &'static str,
    pub lore: [&'static str; 3],
}

pub const fn towerwars_size_options() -> [TowerWarsSizeOption; 2] {
    [
        TowerWarsSizeOption {
            slot: 2,
            size: 1,
            name: "§b§lSolo",
            lore: [
                "§7Chacun sa voie, 2 à 6 joueurs",
                "§7Vous jouez sur votre propre voie.",
                "§a▶ Cliquez pour rejoindre !",
            ],
        },
        TowerWarsSizeOption {
            slot: 6,
            size: 2,
            name: "§b§lDuo",
            lore: [
                "§7Équipes de 2 sur une voie, 4 à 12 joueurs",
                "§7Vous partagez votre voie avec un coéquipier.",
                "§a▶ Cliquez pour rejoindre !",
            ],
        },
    ]
}

pub fn decode_towerwars_size_click(slot: i32) -> Option<u8> {
    let slot = u16::try_from(slot).ok()?;
    towerwars_size_options()
        .iter()
        .find(|o| o.slot == slot)
        .map(|o| o.size)
}

pub fn populate_towerwars_size_menu(inv: &CustomInventory) {
    for option in towerwars_size_options() {
        inv.set(
            option.slot,
            Some(
                ItemStack::new(Material::WhiteWool, option.size)
                    .name(option.name)
                    .lore(option.lore),
            ),
        );
    }
}

pub fn decode_options_click(slot: i32) -> Option<OptionsMenuAction> {
    let slot = u16::try_from(slot).ok()?;
    match slot {
        OPT_SLOT_VISIBILITY => Some(OptionsMenuAction::ToggleVisibility),
        OPT_SLOT_FLIGHT => Some(OptionsMenuAction::ToggleFlight),
        OPT_SLOT_SPEED => Some(OptionsMenuAction::CycleSpeed),
        OPT_SLOT_CHAT_SOUNDS => Some(OptionsMenuAction::ToggleChatSounds),
        OPT_SLOT_BACK => Some(OptionsMenuAction::BackToProfile),
        _ => None,
    }
}

pub fn decode_gadgets_click(slot: i32) -> Option<GadgetsMenuAction> {
    let slot = u16::try_from(slot).ok()?;
    match slot {
        GAD_SLOT_FEATHER => Some(GadgetsMenuAction::Equip(LobbyGadget::FeatherDash)),
        GAD_SLOT_ROCKET => Some(GadgetsMenuAction::Equip(LobbyGadget::FireworkRocket)),
        GAD_SLOT_SHEEP => Some(GadgetsMenuAction::Equip(LobbyGadget::SheepCannon)),
        GAD_SLOT_VORTEX => Some(GadgetsMenuAction::Equip(LobbyGadget::VortexWand)),
        GAD_SLOT_WIND => Some(GadgetsMenuAction::Equip(LobbyGadget::WindCharge)),
        GAD_SLOT_ENDER => Some(GadgetsMenuAction::Equip(LobbyGadget::EnderGrapple)),
        GAD_SLOT_UNEQUIP => Some(GadgetsMenuAction::UnequipGadget),
        GAD_SLOT_TRAIL => Some(GadgetsMenuAction::CycleTrail),
        GAD_SLOT_SHOP => Some(GadgetsMenuAction::OpenShop),
        GAD_SLOT_CLOSE => Some(GadgetsMenuAction::Close),
        _ => None,
    }
}

pub fn player_head(name: &str) -> ItemStack {
    ItemStack::new(Material::PlayerHead, 1).profile(Profile::named(name))
}

pub fn close_item() -> ItemStack {
    ItemStack::new(Material::Barrier, 1).name("§c§lFermer")
}

pub fn back_item(target: &str) -> ItemStack {
    ItemStack::new(Material::Arrow, 1).name(format!("§e§l← Retour {target}"))
}

pub fn apply_lobby_hotbar(player: &Player, prefs: &PlayerPrefs) {
    let menu = ItemStack::new(Material::Compass, 1)
        .name("§6§lMenu principal §7(Clic droit)")
        .lore(["§7Jeux et téléportations."]);

    let profile = player_head(&player.name)
        .name("§e§lMon profil §7(Clic droit)")
        .lore(["§7Infos, statistiques et paramètres."]);

    let collectibles = ItemStack::new(Material::Chest, 1)
        .name("§b§lCollectibles §7(Clic droit)")
        .lore(["§7Gadgets, traînées et boutique."]);

    let equipped_gadget = prefs.equipped_gadget.map(LobbyGadget::hotbar_item);

    let visibility = if prefs.players_visible {
        ItemStack::new(Material::LimeDye, 1)
            .name("§a§lJoueurs : visibles §7(Clic droit)")
            .lore(["§7Cliquez pour masquer les joueurs."])
    } else {
        ItemStack::new(Material::GrayDye, 1)
            .name("§7§lJoueurs : cachés §8(Clic droit)")
            .lore(["§7Cliquez pour afficher les joueurs."])
    };

    let slots = [
        (HOTBAR_MENU_SLOT, Some(menu)),
        (HOTBAR_PROFILE_SLOT, Some(profile)),
        (HOTBAR_COLLECTIBLES_SLOT, Some(collectibles)),
        (HOTBAR_EQUIPPED_GADGET_SLOT, equipped_gadget),
        (HOTBAR_VISIBILITY_SLOT, Some(visibility)),
    ];
    for (slot, item) in slots {
        player
            .inventory
            .set(SlotItem::new(InventorySlot::Hotbar(slot), item));
    }
    for slot in HOTBAR_EMPTY_SLOTS {
        player
            .inventory
            .set(SlotItem::new(InventorySlot::Hotbar(slot), None));
    }
}

pub(crate) fn fill_border(inv: &CustomInventory, rows: u8, accent: Material, neutral: Material) {
    let total = u16::from(rows) * 9;
    let accent_pane = ItemStack::new(accent, 1).name("§8 ");
    let neutral_pane = ItemStack::new(neutral, 1).name("§8 ");
    for slot in 0..total {
        let row = slot / 9;
        let col = slot % 9;
        if row == 0 || row == u16::from(rows) - 1 || col == 0 || col == 8 {
            let pane =
                if (row == 0 || row == u16::from(rows) - 1) && (col == 0 || col == 4 || col == 8) {
                    accent_pane.clone()
                } else {
                    neutral_pane.clone()
                };
            inv.set(slot, Some(pane));
        }
    }
}

fn teleport(material: Material, title: &str, line: &str) -> ItemStack {
    ItemStack::new(material, 1)
        .name(format!("§e{title}"))
        .lore([format!("§7{line}"), "§e▶ Cliquez pour y aller".to_owned()])
}

pub fn populate_games_menu(inv: &CustomInventory, queued: QueueCounts) {
    fill_border(
        inv,
        GAMES_ROWS,
        Material::PurpleStainedGlassPane,
        Material::GrayStainedGlassPane,
    );

    inv.set(
        SLOT_SHEEPWARS,
        Some(
            ItemStack::new(Material::WhiteWool, 1)
                .name("§d§lSheepWars")
                .lore([
                    "§7Deux équipes, des moutons explosifs.".to_owned(),
                    String::new(),
                    format!("§7En jeu : §a{}", queued.sheepwars_playing),
                    format!("§7En file : §d{}", queued.sheepwars),
                    "§a▶ Cliquez pour jouer".to_owned(),
                ])
                .enchant(Enchantment::Unbreaking, 1),
        ),
    );
    inv.set(
        SLOT_TOWERWARS,
        Some(
            ItemStack::new(Material::Crossbow, 1)
                .name("§b§lTowerWars")
                .lore([
                    "§7Tower defense, Solo ou Duo, 2 à 12 joueurs.".to_owned(),
                    String::new(),
                    format!("§7En jeu : §a{}", queued.towerwars_playing),
                    format!("§7En file : §b{}", queued.towerwars),
                    "§a▶ Cliquez pour jouer".to_owned(),
                ])
                .enchant(Enchantment::Unbreaking, 1),
        ),
    );

    inv.set(
        SLOT_TP_SPAWN,
        Some(teleport(
            Material::GoldIngot,
            "Parvis",
            "Haut du tapis rouge.",
        )),
    );
    inv.set(
        SLOT_TP_PORTAL,
        Some(teleport(
            Material::EnderPearl,
            "Place centrale",
            "Devant le Vortex.",
        )),
    );
    inv.set(
        SLOT_TP_CAPITAINERIE,
        Some(teleport(
            Material::Spyglass,
            "Capitainerie",
            "Le Capitaine et les agents IA.",
        )),
    );
    inv.set(
        SLOT_TP_CASTLE,
        Some(teleport(
            Material::StoneBricks,
            "Remparts du château",
            "Vue sur la cité.",
        )),
    );

    inv.set(SLOT_GAMES_CLOSE, Some(close_item()));
}

fn toggle_item(material: Material, label: &str, on: bool, detail: &str) -> ItemStack {
    let (state, action) = if on {
        ("§aactivé", "§e▶ Cliquez pour désactiver")
    } else {
        ("§cdésactivé", "§a▶ Cliquez pour activer")
    };
    let item = ItemStack::new(material, 1)
        .name(format!("§f§l{label} : {state}"))
        .lore([format!("§7{detail}"), String::new(), action.to_owned()]);
    if on {
        item.enchant(Enchantment::Unbreaking, 1)
    } else {
        item
    }
}

pub fn populate_options_menu(inv: &CustomInventory, prefs: &PlayerPrefs) {
    fill_border(
        inv,
        OPTIONS_ROWS,
        Material::OrangeStainedGlassPane,
        Material::GrayStainedGlassPane,
    );

    inv.set(
        OPT_SLOT_VISIBILITY,
        Some(toggle_item(
            if prefs.players_visible {
                Material::LimeDye
            } else {
                Material::GrayDye
            },
            "Joueurs visibles",
            prefs.players_visible,
            "Afficher les autres joueurs du hub.",
        )),
    );
    inv.set(
        OPT_SLOT_FLIGHT,
        Some(toggle_item(
            Material::Feather,
            "Vol",
            prefs.flight_enabled,
            "Double-espace pour voler (/fly).",
        )),
    );

    let speed_mat = match prefs.speed_tier {
        crate::prefs::SpeedTier::Normal => Material::Sugar,
        crate::prefs::SpeedTier::Fast | crate::prefs::SpeedTier::Sprint => Material::RabbitFoot,
    };
    inv.set(
        OPT_SLOT_SPEED,
        Some(
            ItemStack::new(speed_mat, 1)
                .name(format!("§f§lVitesse : {}", prefs.speed_tier.label()))
                .lore([
                    "§7Marche et vol dans le hub.",
                    "",
                    "§e▶ Cliquez pour changer",
                ]),
        ),
    );

    inv.set(
        OPT_SLOT_CHAT_SOUNDS,
        Some(toggle_item(
            if prefs.chat_sounds {
                Material::Bell
            } else {
                Material::NoteBlock
            },
            "Son des mentions",
            prefs.chat_sounds,
            "Un son quand on écrit @VotrePseudo.",
        )),
    );

    inv.set(OPT_SLOT_BACK, Some(back_item("au profil")));
}

pub fn populate_gadgets_menu(
    inv: &CustomInventory,
    prefs: &PlayerPrefs,
    profile: Option<&PlayerProfile>,
) {
    fill_border(
        inv,
        GADGETS_ROWS,
        Material::LightBlueStainedGlassPane,
        Material::GrayStainedGlassPane,
    );

    let gadget_slots = [
        (GAD_SLOT_FEATHER, LobbyGadget::FeatherDash),
        (GAD_SLOT_ROCKET, LobbyGadget::FireworkRocket),
        (GAD_SLOT_SHEEP, LobbyGadget::SheepCannon),
        (GAD_SLOT_VORTEX, LobbyGadget::VortexWand),
        (GAD_SLOT_WIND, LobbyGadget::WindCharge),
        (GAD_SLOT_ENDER, LobbyGadget::EnderGrapple),
    ];

    for (slot, gadget) in gadget_slots {
        let equipped = prefs.equipped_gadget == Some(gadget);
        let usable = gadget_usable(profile, gadget);
        let [line1, line2] = gadget.description();
        let cd_secs = gadget.cooldown().as_secs_f32();
        let action_line = if equipped {
            "§a✔ Équipé".to_owned()
        } else if usable {
            "§e▶ Cliquez pour équiper".to_owned()
        } else {
            let requirement = Catalog::entry(gadget.cosmetic())
                .map_or_else(|| "Verrouillé".to_owned(), |e| unlock_label(e.unlock));
            format!("§c✖ {requirement}")
        };
        let mut item = ItemStack::new(gadget.material(), 1)
            .name(if equipped {
                format!("{} §a§l[ÉQUIPÉ]", gadget.title())
            } else if usable {
                gadget.title().to_owned()
            } else {
                format!("{} §c§l[VERROUILLÉ]", gadget.title())
            })
            .lore([
                line1.to_owned(),
                line2.to_owned(),
                String::new(),
                format!("§7Recharge : §f{cd_secs:.1}s"),
                action_line,
            ]);
        if equipped {
            item = item.enchant(Enchantment::Unbreaking, 1);
        }
        inv.set(slot, Some(item));
    }

    inv.set(
        GAD_SLOT_UNEQUIP,
        Some(
            ItemStack::new(Material::Barrier, 1)
                .name("§c§lRanger le gadget")
                .lore([if prefs.equipped_gadget.is_none() {
                    "§7Aucun gadget équipé."
                } else {
                    "§e▶ Cliquez pour le retirer de la barre"
                }]),
        ),
    );

    inv.set(
        GAD_SLOT_TRAIL,
        Some(
            ItemStack::new(prefs.aura_trail.material(), 1)
                .name(format!("§d§lTraînée : {}", prefs.aura_trail.label()))
                .lore([
                    "§7Particules derrière vous.",
                    "",
                    "§e▶ Cliquez pour changer",
                ]),
        ),
    );

    let coins_line = profile.map_or_else(
        || "§7Coins : §findisponible".to_owned(),
        |p| format!("§6Coins : §f{}", p.coins),
    );
    inv.set(
        GAD_SLOT_SHOP,
        Some(
            ItemStack::new(Material::GoldIngot, 1)
                .name("§a§lBoutique")
                .lore([
                    coins_line,
                    String::new(),
                    "§e▶ Débloquer de nouveaux cosmétiques".to_owned(),
                ]),
        ),
    );

    inv.set(GAD_SLOT_CLOSE, Some(close_item()));
}

type ServerMenuIds = Vec<Option<String>>;
type ProgressMenuEntry = (CustomInventory, ProgressMenuKind, String);

#[derive(Default)]
pub struct MenuManager {
    games_menu: RwLock<Option<CustomInventory>>,
    options_menus: RwLock<HashMap<Uuid, CustomInventory>>,
    gadgets_menus: RwLock<HashMap<Uuid, CustomInventory>>,
    servers_menus: RwLock<HashMap<Uuid, (CustomInventory, ServerMenuIds)>>,
    progress_menus: RwLock<HashMap<Uuid, ProgressMenuEntry>>,
    towerwars_size_menus: RwLock<HashMap<Uuid, CustomInventory>>,
}

impl MenuManager {
    pub async fn init(&self, server: &Server) -> Result<(), Error> {
        let inv = server
            .create_inventory(GAMES_ROWS, "§5§lHypolia §8• §fMenu principal")
            .await?;
        populate_games_menu(&inv, QueueCounts::default());
        *self.games_menu.write().unwrap_or_else(|e| e.into_inner()) = Some(inv);
        Ok(())
    }

    pub fn games_menu(&self) -> Option<CustomInventory> {
        self.games_menu
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn refresh_games_menu(&self, queued: QueueCounts) {
        if let Some(inv) = self.games_menu() {
            populate_games_menu(&inv, queued);
        }
    }

    pub async fn open_games(&self, player: &Player) -> Result<(), Error> {
        if let Some(inv) = self.games_menu() {
            player.inventory.open(&inv).await?;
        }
        Ok(())
    }

    pub async fn open_options(
        &self,
        server: &Server,
        player: &Player,
        prefs: &PlayerPrefs,
    ) -> Result<(), Error> {
        self.discard_options(player.uuid);
        let inv = server
            .create_inventory(OPTIONS_ROWS, "§6§lProfil §8• §fParamètres")
            .await?;
        populate_options_menu(&inv, prefs);
        self.options_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(player.uuid, inv.clone());
        player.inventory.open(&inv).await
    }

    pub fn refresh_open_options(&self, player_uuid: Uuid, prefs: &PlayerPrefs) {
        let inv = self
            .options_menus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&player_uuid)
            .cloned();
        if let Some(inv) = inv {
            populate_options_menu(&inv, prefs);
        }
    }

    pub async fn open_gadgets(
        &self,
        server: &Server,
        player: &Player,
        prefs: &PlayerPrefs,
        profile: Option<&PlayerProfile>,
    ) -> Result<(), Error> {
        self.discard_gadgets(player.uuid);
        let inv = server
            .create_inventory(GADGETS_ROWS, "§b§lHypolia §8• §fCollectibles")
            .await?;
        populate_gadgets_menu(&inv, prefs, profile);
        self.gadgets_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(player.uuid, inv.clone());
        player.inventory.open(&inv).await
    }

    pub fn refresh_open_gadgets(
        &self,
        player_uuid: Uuid,
        prefs: &PlayerPrefs,
        profile: Option<&PlayerProfile>,
    ) {
        let inv = self
            .gadgets_menus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&player_uuid)
            .cloned();
        if let Some(inv) = inv {
            populate_gadgets_menu(&inv, prefs, profile);
        }
    }

    pub async fn open_progress(
        &self,
        server: &Server,
        player: &Player,
        kind: ProgressMenuKind,
        profile: Option<&PlayerProfile>,
        claims: Option<&UserClaims>,
    ) -> Result<(), Error> {
        self.discard_progress(player.uuid);
        let inv = server.create_inventory(kind.rows(), kind.title()).await?;
        let view = ProfileView {
            name: &player.name,
            profile,
            claims,
        };
        populate_progress_menu(&inv, kind, &view);
        self.progress_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(player.uuid, (inv.clone(), kind, player.name.clone()));
        player.inventory.open(&inv).await
    }

    pub fn progress_menu_kind(
        &self,
        player_uuid: Uuid,
        inv_ref: Option<&InventoryRef>,
    ) -> Option<ProgressMenuKind> {
        let custom = inv_ref.and_then(InventoryRef::custom)?;
        self.progress_menus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&player_uuid)
            .filter(|(known, _, _)| known == custom)
            .map(|(_, kind, _)| *kind)
    }

    pub fn show_progress_page(
        &self,
        player_uuid: Uuid,
        kind: ProgressMenuKind,
        profile: Option<&PlayerProfile>,
        claims: Option<&UserClaims>,
    ) {
        let mut guard = self
            .progress_menus
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if let Some((inv, current, name)) = guard.get_mut(&player_uuid) {
            *current = kind;
            let view = ProfileView {
                name,
                profile,
                claims,
            };
            populate_progress_menu(inv, kind, &view);
        }
    }

    pub fn refresh_open_progress(
        &self,
        player_uuid: Uuid,
        profile: Option<&PlayerProfile>,
        claims: Option<&UserClaims>,
    ) {
        let guard = self
            .progress_menus
            .read()
            .unwrap_or_else(|e| e.into_inner());
        if let Some((inv, kind, name)) = guard.get(&player_uuid) {
            let view = ProfileView {
                name,
                profile,
                claims,
            };
            populate_progress_menu(inv, *kind, &view);
        }
    }

    pub fn discard_progress(&self, player_uuid: Uuid) {
        if let Some((removed, _, _)) = self
            .progress_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player_uuid)
        {
            removed.discard();
        }
    }

    pub async fn open_servers(
        &self,
        server: &Server,
        player: &Player,
        lobby_version: Option<&str>,
        snapshot: &FleetStatusSnapshot,
    ) -> Result<(), Error> {
        self.discard_servers(player.uuid);
        let inv = server
            .create_inventory(SERVERS_ROWS, "§d§lHypolia §8• §fServeurs")
            .await?;
        populate_servers_menu(
            &inv,
            lobby_version,
            snapshot.orchestrator_version.as_deref(),
            snapshot.proxy_version.as_deref(),
            &snapshot.servers,
            &snapshot.lobbies,
        );
        self.servers_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                player.uuid,
                (
                    inv.clone(),
                    sorted_server_ids(&snapshot.servers, &snapshot.lobbies),
                ),
            );
        player.inventory.open(&inv).await
    }

    pub fn servers_menu_server_ids(
        &self,
        player_uuid: Uuid,
        inv_ref: Option<&InventoryRef>,
    ) -> Option<ServerMenuIds> {
        let custom = inv_ref.and_then(InventoryRef::custom)?;
        self.servers_menus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&player_uuid)
            .filter(|(known, _)| known == custom)
            .map(|(_, ids)| ids.clone())
    }

    pub async fn open_towerwars_size(&self, server: &Server, player: &Player) -> Result<(), Error> {
        self.discard_towerwars_size(player.uuid);
        let inv = server
            .create_inventory(TOWERWARS_SIZE_ROWS, "§b§lTowerWars §8• §fTaille de partie")
            .await?;
        populate_towerwars_size_menu(&inv);
        self.towerwars_size_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(player.uuid, inv.clone());
        player.inventory.open(&inv).await
    }

    pub fn is_player_towerwars_size_inventory(
        &self,
        player_uuid: Uuid,
        inv_ref: Option<&InventoryRef>,
    ) -> bool {
        let Some(custom) = inv_ref.and_then(InventoryRef::custom) else {
            return false;
        };
        self.towerwars_size_menus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&player_uuid)
            .is_some_and(|known| known == custom)
    }

    pub fn discard_towerwars_size(&self, player_uuid: Uuid) {
        if let Some(removed) = self
            .towerwars_size_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player_uuid)
        {
            removed.discard();
        }
    }

    pub fn discard_servers(&self, player_uuid: Uuid) {
        if let Some((removed, _)) = self
            .servers_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player_uuid)
        {
            removed.discard();
        }
    }

    pub fn is_games_inventory(&self, inv_ref: Option<&InventoryRef>) -> bool {
        let Some(custom) = inv_ref.and_then(InventoryRef::custom) else {
            return false;
        };
        self.games_menu().as_ref() == Some(custom)
    }

    pub fn is_player_options_inventory(
        &self,
        player_uuid: Uuid,
        inv_ref: Option<&InventoryRef>,
    ) -> bool {
        let Some(custom) = inv_ref.and_then(InventoryRef::custom) else {
            return false;
        };
        self.options_menus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&player_uuid)
            .is_some_and(|known| known == custom)
    }

    pub fn is_player_gadgets_inventory(
        &self,
        player_uuid: Uuid,
        inv_ref: Option<&InventoryRef>,
    ) -> bool {
        let Some(custom) = inv_ref.and_then(InventoryRef::custom) else {
            return false;
        };
        self.gadgets_menus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&player_uuid)
            .is_some_and(|known| known == custom)
    }

    pub fn handle_close(&self, player_uuid: Uuid, closed: &InventoryRef) {
        let Some(custom) = closed.custom() else {
            return;
        };
        let mut opt_guard = self
            .options_menus
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if opt_guard.get(&player_uuid) == Some(custom)
            && let Some(removed) = opt_guard.remove(&player_uuid)
        {
            removed.discard();
        }
        drop(opt_guard);

        let mut gad_guard = self
            .gadgets_menus
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if gad_guard.get(&player_uuid) == Some(custom)
            && let Some(removed) = gad_guard.remove(&player_uuid)
        {
            removed.discard();
        }
        drop(gad_guard);

        let mut srv_guard = self
            .servers_menus
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if srv_guard.get(&player_uuid).map(|(inv, _)| inv) == Some(custom)
            && let Some((removed, _)) = srv_guard.remove(&player_uuid)
        {
            removed.discard();
        }
        drop(srv_guard);

        let mut prg_guard = self
            .progress_menus
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if prg_guard.get(&player_uuid).map(|(inv, _, _)| inv) == Some(custom)
            && let Some((removed, _, _)) = prg_guard.remove(&player_uuid)
        {
            removed.discard();
        }
        drop(prg_guard);

        let mut size_guard = self
            .towerwars_size_menus
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if size_guard.get(&player_uuid) == Some(custom)
            && let Some(removed) = size_guard.remove(&player_uuid)
        {
            removed.discard();
        }
    }

    pub fn discard_options(&self, player_uuid: Uuid) {
        if let Some(removed) = self
            .options_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player_uuid)
        {
            removed.discard();
        }
    }

    pub fn discard_gadgets(&self, player_uuid: Uuid) {
        if let Some(removed) = self
            .gadgets_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player_uuid)
        {
            removed.discard();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn towerwars_size_menu_is_one_row_with_symmetric_wool() {
        assert_eq!(TOWERWARS_SIZE_ROWS, 1);
        let options = towerwars_size_options();
        let layout: Vec<(u16, u8)> = options.iter().map(|o| (o.slot, o.size)).collect();
        assert_eq!(layout, vec![(2, 1), (6, 2)]);
        assert!(options.iter().all(|o| o.slot < 9));
    }

    #[test]
    fn towerwars_size_options_have_french_names_and_lore() {
        let options = towerwars_size_options();
        let names: Vec<&str> = options.iter().map(|o| o.name).collect();
        assert_eq!(names, vec!["§b§lSolo", "§b§lDuo"]);
        assert_eq!(options[0].lore[0], "§7Chacun sa voie, 2 à 6 joueurs");
        assert_eq!(
            options[1].lore[0],
            "§7Équipes de 2 sur une voie, 4 à 12 joueurs"
        );
        assert!(options.iter().all(|o| o.lore[1].contains("voie")));
    }

    #[test]
    fn towerwars_size_click_maps_slot_to_match_size() {
        assert_eq!(decode_towerwars_size_click(2), Some(1));
        assert_eq!(decode_towerwars_size_click(6), Some(2));
        assert_eq!(decode_towerwars_size_click(0), None);
        assert_eq!(decode_towerwars_size_click(1), None);
        assert_eq!(decode_towerwars_size_click(4), None);
        assert_eq!(decode_towerwars_size_click(9), None);
        assert_eq!(decode_towerwars_size_click(-1), None);
    }

    #[test]
    fn games_options_and_gadgets_slots_decode_to_expected_actions() {
        assert_eq!(
            decode_games_click(i32::from(SLOT_SHEEPWARS)),
            Some(GamesMenuAction::JoinSheepWars)
        );
        assert_eq!(
            decode_games_click(i32::from(SLOT_TOWERWARS)),
            Some(GamesMenuAction::JoinTowerWars)
        );
        assert_eq!(
            decode_games_click(i32::from(SLOT_TP_PORTAL)),
            Some(GamesMenuAction::TeleportPoi("portal"))
        );
        assert_eq!(
            decode_games_click(i32::from(SLOT_GAMES_CLOSE)),
            Some(GamesMenuAction::Close)
        );
        assert_eq!(decode_games_click(0), None);

        assert_eq!(
            decode_options_click(i32::from(OPT_SLOT_VISIBILITY)),
            Some(OptionsMenuAction::ToggleVisibility)
        );
        assert_eq!(
            decode_options_click(i32::from(OPT_SLOT_FLIGHT)),
            Some(OptionsMenuAction::ToggleFlight)
        );
        assert_eq!(
            decode_options_click(i32::from(OPT_SLOT_SPEED)),
            Some(OptionsMenuAction::CycleSpeed)
        );
        assert_eq!(
            decode_options_click(i32::from(OPT_SLOT_BACK)),
            Some(OptionsMenuAction::BackToProfile)
        );

        assert_eq!(
            decode_gadgets_click(i32::from(GAD_SLOT_FEATHER)),
            Some(GadgetsMenuAction::Equip(LobbyGadget::FeatherDash))
        );
        assert_eq!(
            decode_gadgets_click(i32::from(GAD_SLOT_SHEEP)),
            Some(GadgetsMenuAction::Equip(LobbyGadget::SheepCannon))
        );
        assert_eq!(
            decode_gadgets_click(i32::from(GAD_SLOT_TRAIL)),
            Some(GadgetsMenuAction::CycleTrail)
        );
        assert_eq!(
            decode_gadgets_click(i32::from(GAD_SLOT_SHOP)),
            Some(GadgetsMenuAction::OpenShop)
        );
        assert_eq!(
            decode_gadgets_click(i32::from(GAD_SLOT_UNEQUIP)),
            Some(GadgetsMenuAction::UnequipGadget)
        );
    }

    #[test]
    fn hotbar_layout_matches_the_lobby_design() {
        assert_eq!(HOTBAR_MENU_SLOT, 0);
        assert_eq!(HOTBAR_PROFILE_SLOT, 1);
        assert_eq!(HOTBAR_COLLECTIBLES_SLOT, 4);
        assert_eq!(HOTBAR_EQUIPPED_GADGET_SLOT, HOTBAR_COLLECTIBLES_SLOT + 1);
        assert_eq!(HOTBAR_VISIBILITY_SLOT, 8);
        let mut used = vec![
            HOTBAR_MENU_SLOT,
            HOTBAR_PROFILE_SLOT,
            HOTBAR_COLLECTIBLES_SLOT,
            HOTBAR_EQUIPPED_GADGET_SLOT,
            HOTBAR_VISIBILITY_SLOT,
        ];
        used.extend(HOTBAR_EMPTY_SLOTS);
        used.sort_unstable();
        assert_eq!(used, (0..9).collect::<Vec<u8>>());
    }
}
