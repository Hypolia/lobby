use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use hypolia_protocol::CosmeticId;
use rspigot_plugin::{
    BlockPos, Effect, EffectType, EntityType, Face, Facing, ItemStack, Material, Particle,
    ParticleBurst, Player, Position, Sound, SoundCue, Uuid, Vector,
};

const TRAIL_INTERVAL: Duration = Duration::from_millis(180);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LobbyGadget {
    FeatherDash,
    FireworkRocket,
    SheepCannon,
    VortexWand,
    WindCharge,
    EnderGrapple,
}

impl LobbyGadget {
    pub const ALL: [Self; 6] = [
        Self::FeatherDash,
        Self::FireworkRocket,
        Self::SheepCannon,
        Self::VortexWand,
        Self::WindCharge,
        Self::EnderGrapple,
    ];

    pub const fn material(self) -> Material {
        match self {
            Self::FeatherDash => Material::Feather,
            Self::FireworkRocket => Material::FireworkRocket,
            Self::SheepCannon => Material::PinkWool,
            Self::VortexWand => Material::BlazeRod,
            Self::WindCharge => Material::WindCharge,
            Self::EnderGrapple => Material::EnderPearl,
        }
    }

    pub const fn cosmetic(self) -> CosmeticId {
        match self {
            Self::FeatherDash => CosmeticId::GadgetFeatherDash,
            Self::FireworkRocket => CosmeticId::GadgetFireworkRocket,
            Self::SheepCannon => CosmeticId::GadgetSheepCannon,
            Self::VortexWand => CosmeticId::GadgetVortexWand,
            Self::WindCharge => CosmeticId::GadgetWindCharge,
            Self::EnderGrapple => CosmeticId::GadgetEnderGrapple,
        }
    }

    pub fn from_cosmetic(cosmetic: CosmeticId) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.cosmetic() == cosmetic)
    }

    pub fn from_material(material: &Material) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.material() == *material)
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::FeatherDash => "§b§lPropulseur Éthéré",
            Self::FireworkRocket => "§c§lFusée d'Artifice",
            Self::SheepCannon => "§d§lCanon à Moutons",
            Self::VortexWand => "§5§lBaguette Vortex",
            Self::WindCharge => "§f§lBourrasque de Vent",
            Self::EnderGrapple => "§3§lGrappin Énder",
        }
    }

    pub const fn description(self) -> [&'static str; 2] {
        match self {
            Self::FeatherDash => [
                "§7Vous propulse dans la direction",
                "§7de votre regard avec un nuage léger.",
            ],
            Self::FireworkRocket => [
                "§7Décollage vertical explosif avec",
                "§7fusée et pluie d'étincelles !",
            ],
            Self::SheepCannon => [
                "§7Tire un mouton festif devant vous",
                "§7avant qu'il ne disparaisse en confettis.",
            ],
            Self::VortexWand => [
                "§7Lévitation cosmique et descente douce",
                "§7entourées d'un anneau du Néant.",
            ],
            Self::WindCharge => [
                "§7Libère une onde de vent puissante",
                "§7pour bondir d'un toit à l'autre.",
            ],
            Self::EnderGrapple => [
                "§7Téléporte instantanément sur le bloc",
                "§7visé (ou propulse vers l'horizon).",
            ],
        }
    }

    pub const fn cooldown(self) -> Duration {
        match self {
            Self::FeatherDash => Duration::from_millis(1400),
            Self::FireworkRocket => Duration::from_millis(2800),
            Self::SheepCannon => Duration::from_millis(2400),
            Self::VortexWand => Duration::from_millis(3800),
            Self::WindCharge => Duration::from_millis(1800),
            Self::EnderGrapple => Duration::from_millis(2200),
        }
    }

    pub fn hotbar_item(self) -> ItemStack {
        let [line1, line2] = self.description();
        ItemStack::new(self.material(), 1)
            .name(format!("{} §7(Clic droit)", self.title()))
            .lore([line1, line2, "", "§e▶ Clic droit pour activer !"])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AuraTrail {
    #[default]
    None,
    Flame,
    Heart,
    Magic,
    Emerald,
}

impl AuraTrail {
    pub const fn next(self) -> Self {
        match self {
            Self::None => Self::Flame,
            Self::Flame => Self::Heart,
            Self::Heart => Self::Magic,
            Self::Magic => Self::Emerald,
            Self::Emerald => Self::None,
        }
    }

    pub const fn cosmetic(self) -> Option<CosmeticId> {
        match self {
            Self::None => None,
            Self::Flame => Some(CosmeticId::TrailFlame),
            Self::Heart => Some(CosmeticId::TrailHeart),
            Self::Magic => Some(CosmeticId::TrailMagic),
            Self::Emerald => Some(CosmeticId::TrailEmerald),
        }
    }

    pub fn from_cosmetic(cosmetic: CosmeticId) -> Option<Self> {
        [Self::Flame, Self::Heart, Self::Magic, Self::Emerald]
            .into_iter()
            .find(|t| t.cosmetic() == Some(cosmetic))
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "§7Aucune",
            Self::Flame => "§6Flammes Nordiques",
            Self::Heart => "§dCœurs Festifs",
            Self::Magic => "§bÉtoiles du Vortex",
            Self::Emerald => "§aÉtincelles d'Émeraude",
        }
    }

    pub const fn particle(self) -> Option<Particle> {
        match self {
            Self::None => None,
            Self::Flame => Some(Particle::SmallFlame),
            Self::Heart => Some(Particle::Heart),
            Self::Magic => Some(Particle::EndRod),
            Self::Emerald => Some(Particle::HappyVillager),
        }
    }

    pub const fn material(self) -> Material {
        match self {
            Self::None => Material::GlassBottle,
            Self::Flame => Material::BlazePowder,
            Self::Heart => Material::Poppy,
            Self::Magic => Material::AmethystCluster,
            Self::Emerald => Material::Emerald,
        }
    }
}

pub fn forward_vector(yaw_deg: f32, pitch_deg: f32) -> Vector {
    let yaw = f64::from(yaw_deg).to_radians();
    let pitch = f64::from(pitch_deg).to_radians();
    let xz = pitch.cos();
    Vector {
        x: -yaw.sin() * xz,
        y: -pitch.sin(),
        z: yaw.cos() * xz,
    }
}

pub fn teleport_destination(pos: BlockPos, face: Face) -> Position {
    let (dx, dy, dz) = match face {
        Face::Up => (0.0, 1.0, 0.0),
        Face::Down => (0.0, -2.0, 0.0),
        Face::North => (0.0, 0.2, -1.0),
        Face::South => (0.0, 0.2, 1.0),
        Face::West => (-1.0, 0.2, 0.0),
        Face::East => (1.0, 0.2, 0.0),
        _ => (0.0, 1.0, 0.0),
    };
    Position {
        x: f64::from(pos.x) + 0.5 + dx,
        y: f64::from(pos.y) + dy,
        z: f64::from(pos.z) + 0.5 + dz,
    }
}

#[derive(Default)]
pub struct GadgetManager {
    cooldowns: RwLock<HashMap<(Uuid, LobbyGadget), Instant>>,
    trail_ticks: RwLock<HashMap<Uuid, Instant>>,
}

impl GadgetManager {
    pub fn clear_player(&self, uuid: Uuid) {
        self.cooldowns
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(id, _), _| *id != uuid);
        self.trail_ticks
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&uuid);
    }

    pub fn try_consume_cooldown(&self, uuid: Uuid, gadget: LobbyGadget) -> Result<(), Duration> {
        let now = Instant::now();
        let cooldown = gadget.cooldown();
        let mut guard = self.cooldowns.write().unwrap_or_else(|e| e.into_inner());
        if let Some(last) = guard.get(&(uuid, gadget)) {
            let elapsed = now.saturating_duration_since(*last);
            if elapsed < cooldown {
                return Err(cooldown - elapsed);
            }
        }
        guard.insert((uuid, gadget), now);
        Ok(())
    }

    pub fn emit_trail_if_due(&self, player: &Player, trail: AuraTrail) {
        let Some(particle) = trail.particle() else {
            return;
        };
        let now = Instant::now();
        {
            let mut guard = self.trail_ticks.write().unwrap_or_else(|e| e.into_inner());
            if let Some(last) = guard.get(&player.uuid)
                && now.saturating_duration_since(*last) < TRAIL_INTERVAL
            {
                return;
            }
            guard.insert(player.uuid, now);
        }
        let pos = Position {
            x: player.position.x,
            y: player.position.y + 0.25,
            z: player.position.z,
        };
        player.world.particle.spawn(
            ParticleBurst::new(particle, pos)
                .count(3)
                .spread(Vector {
                    x: 0.25,
                    y: 0.15,
                    z: 0.25,
                })
                .speed(0.01),
        );
    }

    pub fn activate(&self, player: Player, gadget: LobbyGadget) {
        if let Err(wait) = self.try_consume_cooldown(player.uuid, gadget) {
            player
                .sound
                .play(SoundCue::new(Sound::UiButtonClick).volume(0.5).pitch(0.6));
            player.action_bar.send(format!(
                "{} §8• §eRecharge {:.1}s",
                gadget.title(),
                wait.as_secs_f32()
            ));
            return;
        }

        tokio::spawn(async move {
            let dir = forward_vector(player.rotation.yaw, player.rotation.pitch);
            let origin = Position {
                x: player.position.x,
                y: player.position.y + 0.4,
                z: player.position.z,
            };

            match gadget {
                LobbyGadget::FeatherDash => {
                    let boost = Vector {
                        x: dir.x * 1.45,
                        y: (dir.y * 0.8 + 0.55).clamp(0.45, 1.15),
                        z: dir.z * 1.45,
                    };
                    player.movement.set_velocity(boost);
                    player.sound.play(
                        SoundCue::new(Sound::EntityBatTakeoff)
                            .volume(0.85)
                            .pitch(1.25),
                    );
                    player.world.particle.spawn(
                        ParticleBurst::new(Particle::Cloud, origin)
                            .count(16)
                            .spread(Vector {
                                x: 0.35,
                                y: 0.2,
                                z: 0.35,
                            })
                            .speed(0.06),
                    );
                    player.world.particle.spawn(
                        ParticleBurst::new(Particle::EndRod, origin)
                            .count(8)
                            .spread(Vector {
                                x: 0.3,
                                y: 0.25,
                                z: 0.3,
                            })
                            .speed(0.05),
                    );
                    player.action_bar.send("§b✈ Propulseur Éthéré activé !");
                }
                LobbyGadget::FireworkRocket => {
                    player.movement.set_velocity(Vector {
                        x: dir.x * 0.45,
                        y: 1.38,
                        z: dir.z * 0.45,
                    });
                    player.sound.play(
                        SoundCue::new(Sound::EntityFireworkRocketLaunch)
                            .volume(0.95)
                            .pitch(1.1),
                    );
                    player.world.particle.spawn(
                        ParticleBurst::new(Particle::Firework, origin)
                            .count(28)
                            .spread(Vector {
                                x: 0.4,
                                y: 0.3,
                                z: 0.4,
                            })
                            .speed(0.18),
                    );
                    player.world.particle.spawn(
                        ParticleBurst::new(Particle::Flame, origin)
                            .count(14)
                            .spread(Vector {
                                x: 0.25,
                                y: 0.2,
                                z: 0.25,
                            })
                            .speed(0.08),
                    );
                    let world = player.world.clone();
                    if let Ok(rocket) = world.spawn(EntityType::FireworkRocket, origin).await {
                        rocket.set_velocity(Vector {
                            x: dir.x * 0.2,
                            y: 0.7,
                            z: dir.z * 0.2,
                        });
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(1200)).await;
                            rocket.remove();
                        });
                    }
                    player.action_bar.send("§c🎆 Fusée d'Artifice lancée !");
                }
                LobbyGadget::SheepCannon => {
                    let spawn_pos = Position {
                        x: player.position.x + dir.x * 1.2,
                        y: player.position.y + 1.1,
                        z: player.position.z + dir.z * 1.2,
                    };
                    player.sound.play(
                        SoundCue::new(Sound::EntitySheepAmbient)
                            .volume(0.95)
                            .pitch(1.25),
                    );
                    player.sound.play(
                        SoundCue::new(Sound::EntityFireworkRocketShoot)
                            .volume(0.75)
                            .pitch(1.3),
                    );
                    player.world.particle.spawn(
                        ParticleBurst::new(Particle::Heart, spawn_pos)
                            .count(10)
                            .spread(Vector {
                                x: 0.35,
                                y: 0.35,
                                z: 0.35,
                            })
                            .speed(0.05),
                    );
                    if let Ok(sheep) = player.world.spawn(EntityType::Sheep, spawn_pos).await {
                        sheep.set_velocity(Vector {
                            x: dir.x * 1.8,
                            y: (dir.y * 1.2 + 0.45).max(0.25),
                            z: dir.z * 1.8,
                        });
                        let world = player.world.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(2200)).await;
                            if let Ok(state) = sheep.get_state().await {
                                let end = state.entity.position;
                                world.particle.spawn(
                                    ParticleBurst::new(Particle::Cloud, end)
                                        .count(18)
                                        .spread(Vector {
                                            x: 0.4,
                                            y: 0.4,
                                            z: 0.4,
                                        })
                                        .speed(0.08),
                                );
                                world.sound.play(
                                    SoundCue::new(Sound::EntityFireworkRocketBlast)
                                        .volume(0.8)
                                        .pitch(1.4),
                                );
                            }
                            sheep.remove();
                        });
                    }
                    player.action_bar.send("§d🐑 Mouton propulsé !");
                }
                LobbyGadget::VortexWand => {
                    player
                        .effects
                        .add(Effect::new(EffectType::Levitation, 26, 2));
                    player
                        .effects
                        .add(Effect::new(EffectType::SlowFalling, 85, 0));
                    player.sound.play(
                        SoundCue::new(Sound::EntityIllusionerCastSpell)
                            .volume(0.9)
                            .pitch(1.2),
                    );
                    player.world.particle.spawn(
                        ParticleBurst::new(Particle::Portal, origin)
                            .count(30)
                            .spread(Vector {
                                x: 0.65,
                                y: 0.6,
                                z: 0.65,
                            })
                            .speed(0.25),
                    );
                    player.world.particle.spawn(
                        ParticleBurst::new(Particle::Witch, origin)
                            .count(16)
                            .spread(Vector {
                                x: 0.5,
                                y: 0.5,
                                z: 0.5,
                            })
                            .speed(0.1),
                    );
                    player.action_bar.send("§5✦ Lévitation du Vortex !");
                }
                LobbyGadget::WindCharge => {
                    let burst = Vector {
                        x: dir.x * 1.15,
                        y: (dir.y * 0.7 + 0.95).clamp(0.75, 1.35),
                        z: dir.z * 1.15,
                    };
                    player.movement.set_velocity(burst);
                    player.sound.play(
                        SoundCue::new(Sound::EntityWindChargeWindBurst)
                            .volume(0.95)
                            .pitch(1.1),
                    );
                    player.world.particle.spawn(
                        ParticleBurst::new(Particle::GustEmitterSmall, origin)
                            .count(6)
                            .spread(Vector {
                                x: 0.4,
                                y: 0.25,
                                z: 0.4,
                            })
                            .speed(0.05),
                    );
                    player.world.particle.spawn(
                        ParticleBurst::new(Particle::Cloud, origin)
                            .count(14)
                            .spread(Vector {
                                x: 0.45,
                                y: 0.2,
                                z: 0.45,
                            })
                            .speed(0.08),
                    );
                    player.action_bar.send("§f💨 Bourrasque de Vent !");
                }
                LobbyGadget::EnderGrapple => {
                    player.world.particle.spawn(
                        ParticleBurst::new(Particle::ReversePortal, origin)
                            .count(18)
                            .spread(Vector {
                                x: 0.35,
                                y: 0.4,
                                z: 0.35,
                            })
                            .speed(0.08),
                    );
                    if let Ok(Some(hit)) = player.target_block(28.0).await {
                        let dest = teleport_destination(hit.block.pos, hit.face);
                        let facing = Facing::new(dest, player.rotation);
                        if player.location.teleport_facing(facing).await.is_ok() {
                            player.sound.play(
                                SoundCue::new(Sound::EntityEndermanTeleport)
                                    .volume(0.9)
                                    .pitch(1.15),
                            );
                            player.world.particle.spawn(
                                ParticleBurst::new(Particle::Portal, dest)
                                    .count(24)
                                    .spread(Vector {
                                        x: 0.4,
                                        y: 0.5,
                                        z: 0.4,
                                    })
                                    .speed(0.15),
                            );
                            player
                                .action_bar
                                .send("§3⚡ Téléporté via le Grappin Énder !");
                            return;
                        }
                    }
                    // No solid block targeted: propel smoothly forward like an ender dash
                    player.movement.set_velocity(Vector {
                        x: dir.x * 1.6,
                        y: (dir.y * 0.9 + 0.5).clamp(0.35, 1.15),
                        z: dir.z * 1.6,
                    });
                    player.sound.play(
                        SoundCue::new(Sound::EntityEndermanTeleport)
                            .volume(0.8)
                            .pitch(1.3),
                    );
                    player.action_bar.send("§3⚡ Élan Énder vers l'avant !");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use hypolia_progression::Catalog;
    use hypolia_protocol::CosmeticSlot;

    use super::*;

    #[test]
    fn forward_vector_faces_south_at_zero_yaw_and_pitch() {
        let vec = forward_vector(0.0, 0.0);
        assert!(vec.x.abs() < 1e-6);
        assert!(vec.y.abs() < 1e-6);
        assert!((vec.z - 1.0).abs() < 1e-6);
    }

    #[test]
    fn teleport_destination_places_player_safely_on_top_face() {
        let dest = teleport_destination(
            BlockPos {
                x: 10,
                y: 24,
                z: -5,
            },
            Face::Up,
        );
        assert_eq!(
            dest,
            Position {
                x: 10.5,
                y: 25.0,
                z: -4.5,
            }
        );
    }

    #[test]
    fn cooldown_blocks_rapid_reuse_per_player_and_gadget() {
        let mgr = GadgetManager::default();
        let alice = Uuid::new_v4();
        assert!(
            mgr.try_consume_cooldown(alice, LobbyGadget::FeatherDash)
                .is_ok()
        );
        assert!(
            mgr.try_consume_cooldown(alice, LobbyGadget::FeatherDash)
                .is_err()
        );
        assert!(
            mgr.try_consume_cooldown(alice, LobbyGadget::WindCharge)
                .is_ok()
        );
    }

    #[test]
    fn every_gadget_maps_to_a_distinct_gadget_cosmetic_and_back() {
        let mut seen = HashSet::new();
        for gadget in LobbyGadget::ALL {
            let id = gadget.cosmetic();
            assert!(seen.insert(id));
            assert_eq!(
                Catalog::entry(id).map(|e| e.slot),
                Some(CosmeticSlot::LobbyGadget)
            );
            assert_eq!(LobbyGadget::from_cosmetic(id), Some(gadget));
        }
        assert_eq!(LobbyGadget::from_cosmetic(CosmeticId::TrailFlame), None);
    }

    #[test]
    fn every_trail_maps_to_a_trail_cosmetic_and_back() {
        assert_eq!(AuraTrail::None.cosmetic(), None);
        for trail in [
            AuraTrail::Flame,
            AuraTrail::Heart,
            AuraTrail::Magic,
            AuraTrail::Emerald,
        ] {
            let id = trail.cosmetic().expect("trail has a cosmetic");
            assert_eq!(
                Catalog::entry(id).map(|e| e.slot),
                Some(CosmeticSlot::LobbyTrail)
            );
            assert_eq!(AuraTrail::from_cosmetic(id), Some(trail));
        }
        assert_eq!(AuraTrail::from_cosmetic(CosmeticId::GadgetWindCharge), None);
    }

    #[test]
    fn aura_trail_cycles_cleanly() {
        let mut t = AuraTrail::None;
        t = t.next();
        assert_eq!(t, AuraTrail::Flame);
        t = t.next().next().next().next();
        assert_eq!(t, AuraTrail::None);
    }
}
