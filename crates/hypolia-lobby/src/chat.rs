use std::collections::{HashMap, HashSet};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use rspigot_plugin::{Click, Part, Player, RichMessage, Sound, SoundCue, Uuid};

use crate::auth::{Rank, UserClaims};
use crate::prefs::PrefsStore;
use crate::profile::level_prefix;

const CHAT_COOLDOWN: Duration = Duration::from_millis(1000);

pub fn highlight_mentions(text: &str, online_names: &[String]) -> (String, HashSet<String>) {
    let mut mentioned = HashSet::new();
    let mut rendered_words = Vec::new();

    for token in text.split(' ') {
        if let Some(after_at) = token.strip_prefix('@') {
            let clean: String = after_at
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect();
            let trailing = &after_at[clean.len()..];
            if !clean.is_empty()
                && let Some(canonical) =
                    online_names.iter().find(|n| n.eq_ignore_ascii_case(&clean))
            {
                mentioned.insert(canonical.clone());
                rendered_words.push(format!("§e§l@{canonical}§f{trailing}"));
                continue;
            }
        }
        rendered_words.push(token.to_owned());
    }

    (rendered_words.join(" "), mentioned)
}

#[derive(Debug, Clone, Copy)]
pub struct ChatBadge {
    pub rank: Rank,
    pub level: Option<u32>,
}

pub fn format_public_chat(
    sender_name: &str,
    rank: Rank,
    level: Option<u32>,
    claims: Option<&UserClaims>,
    rendered_body: &str,
) -> RichMessage {
    let fk_hover = match claims {
        Some(c) => format!("§a✔ Identité FerrisKey : §f{}", c.username),
        None => "§7Compte FerrisKey non lié".to_owned(),
    };

    let mut msg = RichMessage::new().part(Part::new(format!(
        "{}{} ",
        level_prefix(level),
        rank.chat_badge()
    )));

    if let Some(c) = claims {
        msg = msg.part(
            Part::new("§a✔ ")
                .hover(format!(
                    "§a§lCompte vérifié FerrisKey\n§7Utilisateur : §f{}\n§7Rôles : §b{}",
                    c.username,
                    if c.roles.is_empty() {
                        "membre".to_owned()
                    } else {
                        c.roles.join(", ")
                    }
                ))
                .click(Click::SuggestCommand("/whoami".to_owned())),
        );
    }

    msg.part(
        Part::new(format!("§f{sender_name}"))
            .hover(format!(
                "§6§l{sender_name}\n§7Grade : {}\n{fk_hover}\n\n§eCliquez pour envoyer un message privé",
                rank.sidebar_label()
            ))
            .click(Click::SuggestCommand(format!("/mp {sender_name} "))),
    )
    .text(format!(" §8» §f{rendered_body}"))
}

#[derive(Default)]
pub struct ChatSystem {
    last_chat: RwLock<HashMap<Uuid, Instant>>,
    reply_targets: RwLock<HashMap<Uuid, Uuid>>,
}

impl ChatSystem {
    pub fn check_cooldown(&self, uuid: Uuid, rank: Rank) -> Result<(), Duration> {
        if rank.bypasses_chat_cooldown() {
            return Ok(());
        }
        let now = Instant::now();
        let mut guard = self.last_chat.write().unwrap_or_else(|e| e.into_inner());
        if let Some(prev) = guard.get(&uuid) {
            let elapsed = now.saturating_duration_since(*prev);
            if elapsed < CHAT_COOLDOWN {
                return Err(CHAT_COOLDOWN - elapsed);
            }
        }
        guard.insert(uuid, now);
        Ok(())
    }

    pub fn broadcast_chat(
        &self,
        sender: &Player,
        badge: ChatBadge,
        claims: Option<&UserClaims>,
        raw_message: &str,
        recipients: &[Player],
        prefs: &PrefsStore,
    ) {
        let names: Vec<String> = recipients.iter().map(|p| p.name.clone()).collect();
        let (rendered, mentioned) = highlight_mentions(raw_message, &names);
        let rich = format_public_chat(&sender.name, badge.rank, badge.level, claims, &rendered);

        tracing::info!("[CHAT] {} ({:?}): {}", sender.name, badge.rank, raw_message);

        for target in recipients {
            target.message.send_rich(rich.clone());
            if target.uuid != sender.uuid
                && mentioned.contains(&target.name)
                && prefs.get(target.uuid).chat_sounds
            {
                target.sound.play(
                    SoundCue::new(Sound::BlockNoteBlockPling)
                        .volume(0.9)
                        .pitch(1.6),
                );
            }
        }
    }

    pub fn send_whisper(
        &self,
        sender: &Player,
        recipient: &Player,
        body: &str,
        prefs: &PrefsStore,
    ) {
        {
            let mut guard = self
                .reply_targets
                .write()
                .unwrap_or_else(|e| e.into_inner());
            guard.insert(sender.uuid, recipient.uuid);
            guard.insert(recipient.uuid, sender.uuid);
        }

        sender.message.send_rich(
            RichMessage::new()
                .text("§d§lMP §7vers ")
                .part(
                    Part::new(format!("§f{}", recipient.name))
                        .click(Click::SuggestCommand(format!("/mp {} ", recipient.name))),
                )
                .text(format!(" §8» §d{body}")),
        );

        recipient.message.send_rich(
            RichMessage::new()
                .text("§d§lMP §7de ")
                .part(
                    Part::new(format!("§f{}", sender.name))
                        .hover("§eCliquez pour répondre")
                        .click(Click::SuggestCommand(format!("/mp {} ", sender.name))),
                )
                .text(format!(" §8» §d{body}")),
        );

        if prefs.get(recipient.uuid).chat_sounds {
            recipient.sound.play(
                SoundCue::new(Sound::EntityExperienceOrbPickup)
                    .volume(0.7)
                    .pitch(1.3),
            );
        }
    }

    pub fn reply_partner(&self, sender: Uuid) -> Option<Uuid> {
        self.reply_targets
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&sender)
            .copied()
    }

    pub fn clear_player(&self, uuid: Uuid) {
        self.last_chat
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&uuid);
        self.reply_targets
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&uuid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_highlight_online_players_case_insensitively_with_punctuation() {
        let online = vec!["Nathael".to_owned(), "Astrid".to_owned()];
        let (rendered, mentioned) = highlight_mentions(
            "Salut @nathael, tu viens avec @astrid! et @inconnu ?",
            &online,
        );
        assert!(rendered.contains("§e§l@Nathael§f,"));
        assert!(rendered.contains("§e§l@Astrid§f!"));
        assert!(rendered.contains("@inconnu"));
        assert_eq!(mentioned.len(), 2);
        assert!(mentioned.contains("Nathael"));
        assert!(mentioned.contains("Astrid"));
    }

    #[test]
    fn chat_badge_is_preceded_by_the_network_level_once_known() {
        let with_level = format_public_chat("Abe", Rank::Vip, Some(12), None, "salut");
        let without = format_public_chat("Abe", Rank::Vip, None, None, "salut");
        let first = |msg: &RichMessage| format!("{msg:?}");
        assert!(first(&with_level).contains("§7[12✫] §6§l[VIP] "));
        assert!(!first(&without).contains("✫"));
    }

    #[test]
    fn cooldown_blocks_rapid_messages_for_guests_but_not_admins() {
        let chat = ChatSystem::default();
        let guest = Uuid::new_v4();
        assert!(chat.check_cooldown(guest, Rank::Guest).is_ok());
        assert!(chat.check_cooldown(guest, Rank::Guest).is_err());

        let admin = Uuid::new_v4();
        assert!(chat.check_cooldown(admin, Rank::Admin).is_ok());
        assert!(chat.check_cooldown(admin, Rank::Admin).is_ok());
    }
}
