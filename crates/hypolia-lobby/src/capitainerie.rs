use std::collections::HashSet;
use std::time::Duration;

use futures_util::StreamExt;
use rspigot_plugin::{Click, Dialog, DialogButton, Part, RichMessage};
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

pub const CAPTAIN_DISPLAY_NAME: &str = "Capitaine";
pub const RUN_BUTTON_PREFIX: &str = "hypolia:cap_run/";
pub const RESULT_BUTTON_PREFIX: &str = "hypolia:cap_result/";
pub const CANCEL_BUTTON_ID: &str = "hypolia:cap_cancel";
pub const LOGIN_BUTTON_ID: &str = "hypolia:login_now";
pub const CAP_LOGIN_BUTTON_ID: &str = "hypolia:cap_login_now";

const BAR_WIDTH: u32 = 5;
const TITLE_WIDTH: usize = 32;
const MAX_NPC_NAME: usize = 16;

#[derive(Debug, Clone, Deserialize)]
pub struct AgentList {
    pub items: Vec<TeamAgent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TeamAgent {
    pub id: String,
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub paused: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActiveRuns {
    pub items: Vec<ActiveRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ActiveRun {
    pub run: RunSummary,
    #[serde(default)]
    pub current_step: Option<CurrentStep>,
    #[serde(default)]
    pub steps_done: u32,
    #[serde(default)]
    pub steps_total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RunSummary {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub card_id: Option<String>,
    #[serde(default)]
    pub agent: Option<AgentRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentRef {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CurrentStep {
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentRunOutcome {
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub skipped: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CardCreated {
    pub id: String,
    pub version: i64,
    #[serde(default)]
    pub agent_run: Option<AgentRunOutcome>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunStarted {
    pub run_id: String,
    #[serde(default)]
    pub card_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RunDetail {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub card_id: Option<String>,
    #[serde(default)]
    pub failure: Option<String>,
    #[serde(default)]
    pub agent: Option<AgentRef>,
}

impl RunDetail {
    pub fn is_terminal(&self) -> bool {
        matches!(self.status.as_str(), "succeeded" | "failed" | "cancelled")
    }

    pub fn is_succeeded(&self) -> bool {
        self.status == "succeeded"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CardComment {
    pub id: String,
    pub body: String,
    #[serde(default)]
    pub author_subject: String,
    #[serde(default)]
    pub author_actor: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CardDetail {
    #[serde(default)]
    pub comments: Vec<CardComment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskResultReport {
    pub run_id: String,
    pub card_id: String,
    pub agent_name: String,
    pub agent_slug: String,
    pub task_title: String,
    pub status: String,
    pub summary_text: String,
    pub console_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ConversationCreated {
    id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatOutcome {
    pub conversation: String,
    pub cards: Vec<(String, String)>,
}

#[derive(Debug, Error)]
pub enum CapitainerieError {
    #[error("Capitainerie injoignable : {0}")]
    Http(#[from] reqwest::Error),
    #[error("statut HTTP {status} : {body}")]
    Status { status: u16, body: String },
    #[error("{code}: {message}")]
    Chat { code: String, message: String },
}

impl CapitainerieError {
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Chat { code, .. } => Some(code.as_str()),
            Self::Status { body, .. } => serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|v| v.get("code")?.as_str().map(str::to_owned))
                .and_then(|owned| match owned.as_str() {
                    "agent_paused" => Some("agent_paused"),
                    "agent_archived" => Some("agent_archived"),
                    "card_without_agent" => Some("card_without_agent"),
                    "agent_without_connector" => Some("agent_without_connector"),
                    "no_provider" => Some("no_provider"),
                    "not_connected" => Some("not_connected"),
                    "execution_already_active" => Some("execution_already_active"),
                    "agent_busy" => Some("agent_busy"),
                    "choose_repository" => Some("choose_repository"),
                    "turn_in_progress" => Some("turn_in_progress"),
                    "conversation_full" => Some("conversation_full"),
                    "captain_not_configured" => Some("captain_not_configured"),
                    "captain_unavailable" => Some("captain_unavailable"),
                    _ => None,
                }),
            Self::Http(_) => None,
        }
    }

    pub fn explain(&self) -> String {
        if let Self::Status {
            status: 401 | 403, ..
        } = self
        {
            return "§cVotre compte FerrisKey n'a pas accès à cette action Capitainerie."
                .to_owned();
        }
        let reason = match self.code() {
            Some("agent_paused") => "l'agent est en pause",
            Some("agent_archived") => "l'agent est archivé",
            Some("card_without_agent") => "la carte n'a pas d'agent",
            Some("agent_without_connector") => "l'agent n'a aucun connecteur",
            Some("no_provider") => "aucun fournisseur de modèle n'est configuré",
            Some("not_connected") => "l'intégration n'est pas connectée",
            Some("execution_already_active" | "agent_busy") => {
                "une exécution est déjà en cours sur cet agent"
            }
            Some("choose_repository") => {
                "l'agent a accès à plusieurs dépôts : choisissez-en un dans Capitainerie"
            }
            Some("turn_in_progress") => "le Capitaine répond déjà",
            Some("conversation_full") => "la conversation est pleine, tapez quitter puis reprenez",
            Some("captain_not_configured" | "captain_unavailable") => {
                "le Capitaine est actuellement indisponible"
            }
            _ => match self {
                Self::Chat { message, .. } if !message.trim().is_empty() => message.trim(),
                _ => "Capitainerie a refusé ou interrompu la demande",
            },
        };
        format!("§c{reason}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatEvent {
    Delta(String),
    Card { id: String, title: String },
    Done,
    Error { code: String, message: String },
}

#[derive(Default)]
pub struct SseBuffer {
    pending: String,
}

impl SseBuffer {
    pub fn push(&mut self, chunk: &str) -> Vec<ChatEvent> {
        self.pending.push_str(&chunk.replace("\r\n", "\n"));
        let mut out = Vec::new();
        while let Some(sep) = self.pending.find("\n\n") {
            let block: String = self.pending.drain(..sep).collect();
            self.pending.drain(..2);
            if let Some(parsed) = Self::parse_block(&block) {
                out.push(parsed);
            }
        }
        out
    }

    fn parse_block(block: &str) -> Option<ChatEvent> {
        let mut event_name = None;
        let mut data_lines = Vec::new();
        for line in block.lines() {
            if line.starts_with(':') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("event:") {
                event_name = Some(rest.trim());
            } else if let Some(rest) = line.strip_prefix("data:") {
                data_lines.push(rest.trim_start());
            }
        }
        let kind = event_name?;
        let raw_data = data_lines.join("\n");
        let parsed: serde_json::Value = serde_json::from_str(&raw_data).ok()?;
        match kind {
            "delta" => Some(ChatEvent::Delta(parsed.get("text")?.as_str()?.to_owned())),
            "card" => Some(ChatEvent::Card {
                id: parsed.get("card_id")?.as_str()?.to_owned(),
                title: parsed.get("title")?.as_str()?.to_owned(),
            }),
            "done" => Some(ChatEvent::Done),
            "error" => Some(ChatEvent::Error {
                code: parsed
                    .get("code")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("error")
                    .to_owned(),
                message: parsed
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("erreur inconnue")
                    .to_owned(),
            }),
            _ => None,
        }
    }
}

pub struct CapitainerieClient {
    base_url: String,
    http: reqwest::Client,
}

impl CapitainerieClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            http,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{path}", self.base_url)
    }

    async fn checked(response: reqwest::Response) -> Result<reqwest::Response, CapitainerieError> {
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        Err(CapitainerieError::Status {
            status,
            body: body.chars().take(500).collect(),
        })
    }

    pub async fn agents(&self, bearer: &str) -> Result<Vec<TeamAgent>, CapitainerieError> {
        let resp = self
            .http
            .get(self.url("/team/agents"))
            .bearer_auth(bearer)
            .send()
            .await?;
        let list: AgentList = Self::checked(resp).await?.json().await?;
        Ok(list.items)
    }

    pub async fn active_runs(&self, bearer: &str) -> Result<Vec<ActiveRun>, CapitainerieError> {
        let resp = self
            .http
            .get(self.url("/runs/active"))
            .bearer_auth(bearer)
            .send()
            .await?;
        let runs: ActiveRuns = Self::checked(resp).await?.json().await?;
        Ok(runs.items)
    }

    pub async fn run(&self, bearer: &str, run_id: &str) -> Result<RunDetail, CapitainerieError> {
        let resp = self
            .http
            .get(self.url(&format!("/runs/{run_id}")))
            .bearer_auth(bearer)
            .send()
            .await?;
        Ok(Self::checked(resp).await?.json().await?)
    }

    pub async fn card(&self, bearer: &str, card_id: &str) -> Result<CardDetail, CapitainerieError> {
        let resp = self
            .http
            .get(self.url(&format!("/cards/{card_id}")))
            .bearer_auth(bearer)
            .send()
            .await?;
        Ok(Self::checked(resp).await?.json().await?)
    }

    pub async fn start_task(
        &self,
        bearer: &str,
        agent_id: &str,
        title: &str,
        description: &str,
    ) -> Result<RunStarted, CapitainerieError> {
        let create_resp = self
            .http
            .post(self.url("/cards"))
            .bearer_auth(bearer)
            .json(&json!({
                "title": title,
                "description": description,
                "assignee_agent_id": agent_id,
                "priority": "normal",
            }))
            .send()
            .await?;
        let card: CardCreated = Self::checked(create_resp).await?.json().await?;
        if let Some(outcome) = card.agent_run {
            if let Some(run_id) = outcome.run_id {
                return Ok(RunStarted {
                    run_id,
                    card_id: card.id,
                });
            }
            if let Some(code) = outcome.skipped {
                return Err(CapitainerieError::Chat {
                    code,
                    message: outcome.message.unwrap_or_default(),
                });
            }
        }

        let run_resp = self
            .http
            .post(self.url(&format!("/cards/{}/agent-runs", card.id)))
            .bearer_auth(bearer)
            .json(&json!({
                "expected_version": card.version,
                "allow_writes": false,
            }))
            .send()
            .await?;
        let mut started: RunStarted = Self::checked(run_resp).await?.json().await?;
        if started.card_id.is_empty() {
            started.card_id = card.id;
        }
        Ok(started)
    }

    pub async fn chat(
        &self,
        bearer: &str,
        conversation: Option<&str>,
        message: &str,
        mut on_line: impl FnMut(String),
    ) -> Result<ChatOutcome, CapitainerieError> {
        let (conversation_id, payload_msg) = match conversation {
            Some(existing) => (existing.to_owned(), Some(message)),
            None => {
                let resp = self
                    .http
                    .post(self.url("/conversations"))
                    .bearer_auth(bearer)
                    .json(&json!({"message": message}))
                    .send()
                    .await?;
                let created: ConversationCreated = Self::checked(resp).await?.json().await?;
                (created.id, None)
            }
        };

        let resp = self
            .http
            .post(self.url(&format!("/conversations/{conversation_id}/messages")))
            .bearer_auth(bearer)
            .json(&json!({"message": payload_msg}))
            .send()
            .await?;

        let mut stream = Self::checked(resp).await?.bytes_stream();
        let mut buffer = SseBuffer::default();
        let mut line = String::new();
        let mut cards = Vec::new();
        let mut done = false;

        while let Some(chunk) = stream.next().await {
            for event in buffer.push(&String::from_utf8_lossy(&chunk?)) {
                match event {
                    ChatEvent::Delta(text) => {
                        line.push_str(&text);
                        while let Some(end) = line.find('\n') {
                            let complete: String = line.drain(..=end).collect();
                            on_line(complete.trim_end().to_owned());
                        }
                    }
                    ChatEvent::Card { id, title } => cards.push((id, title)),
                    ChatEvent::Done => done = true,
                    ChatEvent::Error { code, message } => {
                        return Err(CapitainerieError::Chat { code, message });
                    }
                }
            }
        }
        if !line.trim().is_empty() {
            on_line(line.trim().to_owned());
        }
        if !done {
            return Err(CapitainerieError::Chat {
                code: "interrupted".to_owned(),
                message: "la réponse s'est interrompue".to_owned(),
            });
        }

        Ok(ChatOutcome {
            conversation: conversation_id,
            cards,
        })
    }
}

pub fn sanitize_npc_name(raw: &str, taken: &HashSet<String>) -> String {
    let mut cleaned = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            cleaned.push(ch);
        } else if (ch == ' ' || ch == '-') && !cleaned.ends_with('_') && !cleaned.is_empty() {
            cleaned.push('_');
        }
        if cleaned.len() == MAX_NPC_NAME {
            break;
        }
    }
    if cleaned.is_empty() {
        cleaned = "Agent".to_owned();
    }
    let lower_taken: HashSet<String> = taken.iter().map(|s| s.to_ascii_lowercase()).collect();
    if !lower_taken.contains(&cleaned.to_ascii_lowercase()) {
        return cleaned;
    }
    for n in 2..1000u32 {
        let suffix = n.to_string();
        let keep = MAX_NPC_NAME.saturating_sub(suffix.len());
        let candidate = format!("{}{suffix}", &cleaned[..cleaned.len().min(keep)]);
        if !lower_taken.contains(&candidate.to_ascii_lowercase()) {
            return candidate;
        }
    }
    "Agent".to_owned()
}

pub fn click_slug(slug: &str) -> String {
    slug.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

pub fn progress_bar(done: u32, total: u32) -> String {
    let filled = if total == 0 || done == 0 {
        0
    } else if done >= total {
        BAR_WIDTH
    } else {
        (done * BAR_WIDTH).div_ceil(total).clamp(1, BAR_WIDTH - 1)
    };
    format!(
        "§a{}§8{}",
        "▰".repeat(filled as usize),
        "▱".repeat((BAR_WIDTH - filled) as usize)
    )
}

pub fn step_name(kind: &str) -> &str {
    match kind {
        "extract" => "extraction",
        "search" => "recherche",
        "analyze" => "analyse",
        "write" => "rédaction",
        "execute" => "exécution",
        "intervene" => "intervention",
        "agent_task" => "tâche",
        other => other,
    }
}

pub fn clip_text(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    if max_chars == 1 {
        return "…".to_owned();
    }
    let head: String = text.chars().take(max_chars - 1).collect();
    format!("{head}…")
}

fn clip(text: &str) -> String {
    clip_text(text, TITLE_WIDTH)
}

pub fn agent_label(agent: &TeamAgent, run: Option<&ActiveRun>) -> String {
    let head = format!("§b§l{}", agent.name);
    if agent.paused {
        return format!("{head}\n§cEn pause");
    }
    let Some(run) = run else {
        return format!("{head}\n§7Disponible §8(Clic droit)");
    };
    let mut lines = vec![head];
    let step = run
        .current_step
        .as_ref()
        .map(|s| format!(" §7· {}", step_name(&s.kind)))
        .unwrap_or_default();
    if run.run.status == "queued" {
        lines.push("§eEn file d'attente".to_owned());
    } else {
        lines.push(format!("§eAu travail{step}"));
    }
    if run.steps_total > 0 {
        lines.push(format!(
            "{} §7{}/{}",
            progress_bar(run.steps_done, run.steps_total),
            run.steps_done,
            run.steps_total
        ));
    }
    if let Some(title) = run.run.title.as_deref().filter(|t| !t.is_empty()) {
        lines.push(format!("§f{}", clip(title)));
    }
    lines.join("\n")
}

pub fn captain_label(online: bool, configured: bool) -> String {
    if !configured {
        format!("§6§l{CAPTAIN_DISPLAY_NAME}\n§7Quartier Général Hypolia")
    } else if online {
        format!("§6§l{CAPTAIN_DISPLAY_NAME}\n§a● En ligne §7(Clic droit)")
    } else {
        format!("§6§l{CAPTAIN_DISPLAY_NAME}\n§e◌ En attente d'un compte FerrisKey")
    }
}

pub fn is_working(run: Option<&ActiveRun>) -> bool {
    run.is_some_and(|r| r.run.status == "running")
}

pub fn extract_agent_reply(card: &CardDetail, agent_slug: &str) -> Option<String> {
    let expected = format!("agent:{}", agent_slug.to_ascii_lowercase());
    if let Some(comment) = card.comments.iter().rev().find(|c| {
        !c.body.trim().is_empty()
            && c.author_actor
                .as_deref()
                .is_some_and(|actor| actor.eq_ignore_ascii_case(&expected))
    }) {
        return Some(comment.body.trim().to_owned());
    }
    card.comments
        .iter()
        .rev()
        .find(|c| {
            !c.body.trim().is_empty()
                && c.author_actor
                    .as_deref()
                    .is_some_and(|actor| actor.starts_with("agent:"))
        })
        .map(|c| c.body.trim().to_owned())
}

fn strip_markdown_links(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(open_bracket) = rest.find('[') {
        out.push_str(&rest[..open_bracket]);
        let after_bracket = &rest[open_bracket + 1..];
        if let Some(close_bracket) = after_bracket.find("](") {
            let label = &after_bracket[..close_bracket];
            let after_paren_open = &after_bracket[close_bracket + 2..];
            if let Some(close_paren) = after_paren_open.find(')') {
                out.push_str(label);
                rest = &after_paren_open[close_paren + 1..];
                continue;
            }
        }
        out.push('[');
        rest = after_bracket;
    }
    out.push_str(rest);
    out
}

fn clean_inline_markdown(input: &str) -> String {
    let unlinked = strip_markdown_links(input);
    unlinked
        .replace("**", "")
        .replace("__", "")
        .replace('`', "")
        .trim()
        .to_owned()
}

fn wrap_plain_line(text: &str, max_width: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in words {
        let word_len = word.chars().count();
        let cur_len = current.chars().count();
        if current.is_empty() {
            if word_len > max_width {
                current.push_str(&clip_text(word, max_width));
            } else {
                current.push_str(word);
            }
        } else if cur_len + 1 + word_len <= max_width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            if word_len > max_width {
                current.push_str(&clip_text(word, max_width));
            } else {
                current.push_str(word);
            }
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

pub fn format_chat_preview(raw: &str, max_lines: usize, max_chars: usize) -> String {
    let mut collected = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("```") {
            continue;
        }
        let cleaned = clean_inline_markdown(trimmed.trim_start_matches(['#', '-', '*', '>', ' ']));
        if cleaned.is_empty() {
            continue;
        }
        collected.push(cleaned);
        if collected.len() >= max_lines {
            break;
        }
    }
    let joined = collected.join(" · ");
    clip_text(&joined, max_chars)
}

pub fn format_dialog_lines(raw: &str, wrap_width: usize, max_lines: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut truncated = false;

    'outer: for raw_line in raw.lines() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with("```") {
            continue;
        }
        if trimmed.is_empty() {
            if !out.is_empty() && !out.last().is_some_and(String::is_empty) {
                if out.len() >= max_lines {
                    truncated = true;
                    break;
                }
                out.push(String::new());
            }
            continue;
        }

        let (prefix, cont_prefix, body_raw) = if let Some(h) = trimmed.strip_prefix("### ") {
            ("§b§l", "§b§l", h)
        } else if let Some(h) = trimmed.strip_prefix("## ") {
            ("§e§l", "§e§l", h)
        } else if let Some(h) = trimmed.strip_prefix("# ") {
            ("§6§l", "§6§l", h)
        } else if let Some(b) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            ("§7• §f", "  §f", b)
        } else {
            ("§f", "§f", trimmed)
        };

        let cleaned = clean_inline_markdown(body_raw);
        if cleaned.is_empty() {
            continue;
        }
        let wrapped = wrap_plain_line(&cleaned, wrap_width);
        for (idx, chunk) in wrapped.into_iter().enumerate() {
            if out.len() >= max_lines {
                truncated = true;
                break 'outer;
            }
            let pfx = if idx == 0 { prefix } else { cont_prefix };
            out.push(format!("{pfx}{chunk}"));
        }
    }

    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    if truncated && !out.is_empty() {
        out.pop();
        out.push("§8… (réponse complète disponible sur Capitainerie)".to_owned());
    }
    if out.is_empty() {
        out.push("§7(Aucun détail textuel retourné)".to_owned());
    }
    out
}

pub fn build_run_completion_message(report: &TaskResultReport, click_key: &str) -> RichMessage {
    let header = match report.status.as_str() {
        "succeeded" => format!("§a§l✔ {} a terminé sa mission !", report.agent_name),
        "cancelled" => format!("§e⚠ {} — mission annulée", report.agent_name),
        _ => format!("§c§l✖ {} — mission échouée", report.agent_name),
    };
    let preview = format_chat_preview(&report.summary_text, 2, 160);

    let mut msg = RichMessage::new()
        .text("§8§m                                                  \n")
        .text(format!("{header}\n"))
        .text(format!(
            "§7Tâche : §f{}\n",
            clip_text(&report.task_title, 72)
        ));

    if !preview.is_empty() {
        msg = msg.text(format!("§7Aperçu : §f{preview}\n"));
    }

    msg = msg.text("  ").part(
        Part::new("§b§l[📖 Lire la réponse]")
            .hover("§7Afficher le rapport complet en jeu")
            .click(Click::Custom(format!("{RESULT_BUTTON_PREFIX}{click_key}"))),
    );

    if let Some(url) = &report.console_url {
        msg = msg.text("  ").part(
            Part::new("§6§l[🌐 Ouvrir sur Capitainerie]")
                .hover("§7Ouvrir la carte dans votre navigateur")
                .click(Click::OpenUrl(url.clone())),
        );
    }

    msg.text("\n§8§m                                                  ")
}

pub fn build_run_result_dialog(report: &TaskResultReport) -> Dialog {
    let dialog_title = match report.status.as_str() {
        "succeeded" => format!("§a✔ {} — Rapport", report.agent_name),
        "cancelled" => format!("§e⚠ {} — Annulé", report.agent_name),
        _ => format!("§c✖ {} — Échec", report.agent_name),
    };
    let mut dialog = Dialog::new(dialog_title)
        .line(format!(
            "§7Mission : §f{}",
            clip_text(&report.task_title, 72)
        ))
        .line("§8────────────────────────");

    for line in format_dialog_lines(&report.summary_text, 56, 20) {
        dialog = dialog.line(line);
    }

    dialog.button(DialogButton::new("§7Fermer", CANCEL_BUTTON_ID))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_chunks_reassemble_deltas_cards_and_done() {
        let mut buf = SseBuffer::default();
        let mut got = Vec::new();
        for chunk in [
            "event: delta\ndata: {\"te",
            "xt\":\"Salut\"}\n\nevent: card\ndata: {\"card_id\":\"c1\",\"title\":\"Audit\"}\n\n",
            "event: done\ndata: {}\n\n",
        ] {
            got.extend(buf.push(chunk));
        }
        assert_eq!(
            got,
            vec![
                ChatEvent::Delta("Salut".to_owned()),
                ChatEvent::Card {
                    id: "c1".to_owned(),
                    title: "Audit".to_owned()
                },
                ChatEvent::Done,
            ]
        );
    }

    #[test]
    fn sanitize_npc_name_respects_16_char_word_limit_and_uniqueness() {
        let taken: HashSet<String> = ["scribe", "scribe2"].map(str::to_owned).into();
        assert_eq!(sanitize_npc_name("Scribe", &taken), "Scribe3");
        let long = sanitize_npc_name("agent-architecture-systeme", &HashSet::new());
        assert!(long.len() <= 16);
        assert!(long.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }

    #[test]
    fn progress_bar_renders_clean_segments() {
        assert_eq!(progress_bar(0, 4), "§a§8▱▱▱▱▱");
        assert_eq!(progress_bar(2, 4), "§a▰▰▰§8▱▱");
        assert_eq!(progress_bar(4, 4), "§a▰▰▰▰▰§8");
    }

    #[tokio::test]
    async fn start_task_uses_auto_started_run_without_conflicting_second_post() {
        use axum::{Json, Router, http::StatusCode, routing::post};

        let app = Router::new()
            .route(
                "/api/v1/cards",
                post(|| async {
                    (
                        StatusCode::CREATED,
                        Json(json!({
                            "id": "card-auto",
                            "version": 4,
                            "agent_run": {
                                "run_id": "run-auto",
                                "skipped": null,
                                "message": null
                            }
                        })),
                    )
                }),
            )
            .route(
                "/api/v1/cards/{id}/agent-runs",
                post(|| async {
                    (
                        StatusCode::CONFLICT,
                        Json(json!({
                            "code": "conflict",
                            "status": 409,
                            "message": "the card changed since it was read"
                        })),
                    )
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        let client = CapitainerieClient::new(format!("http://{addr}"));
        let started = client
            .start_task("tok", "agent-1", "Lister les DOE", "")
            .await
            .expect("auto-started run should succeed without second POST");
        assert_eq!(started.run_id, "run-auto");
        assert_eq!(started.card_id, "card-auto");

        server.abort();
    }

    #[tokio::test]
    async fn start_task_surfaces_skipped_reason_from_create_card() {
        use axum::{Json, Router, http::StatusCode, routing::post};

        let app = Router::new().route(
            "/api/v1/cards",
            post(|| async {
                (
                    StatusCode::CREATED,
                    Json(json!({
                        "id": "card-skip",
                        "version": 1,
                        "agent_run": {
                            "run_id": null,
                            "skipped": "agent_busy",
                            "message": "L'agent travaille déjà"
                        }
                    })),
                )
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        let client = CapitainerieClient::new(format!("http://{addr}"));
        let err = client
            .start_task("tok", "agent-1", "Mission", "")
            .await
            .unwrap_err();
        assert_eq!(
            err.explain(),
            "§cune exécution est déjà en cours sur cet agent"
        );

        server.abort();
    }

    #[test]
    fn extract_agent_reply_prefers_latest_comment_from_assigned_agent() {
        let card = CardDetail {
            comments: vec![
                CardComment {
                    id: "c1".to_owned(),
                    body: "Question joueur".to_owned(),
                    author_subject: "nathaelb".to_owned(),
                    author_actor: Some("user:nathaelb".to_owned()),
                    created_at: "2026-11-19T10:00:00Z".to_owned(),
                },
                CardComment {
                    id: "c2".to_owned(),
                    body: "Première passe".to_owned(),
                    author_subject: "bernard".to_owned(),
                    author_actor: Some("agent:bernard".to_owned()),
                    created_at: "2026-11-19T10:01:00Z".to_owned(),
                },
                CardComment {
                    id: "c3".to_owned(),
                    body: "Liste finale des DOE : Projet A, Projet B".to_owned(),
                    author_subject: "bernard".to_owned(),
                    author_actor: Some("agent:bernard".to_owned()),
                    created_at: "2026-11-19T10:02:00Z".to_owned(),
                },
            ],
        };
        assert_eq!(
            extract_agent_reply(&card, "bernard").as_deref(),
            Some("Liste finale des DOE : Projet A, Projet B")
        );
    }

    #[test]
    fn markdown_formatters_clean_and_wrap_safely() {
        let md = "## Projets Mestier\n- **Projet Alpha** : `DOE_2026.pdf` ([voir](https://example.com))\n- Projet Beta : complet";
        let preview = format_chat_preview(md, 2, 120);
        assert!(preview.contains("Projets Mestier"));
        assert!(preview.contains("Projet Alpha : DOE_2026.pdf (voir)"));
        assert!(!preview.contains("**"));
        assert!(!preview.contains("https://example.com"));

        let lines = format_dialog_lines(md, 40, 10);
        assert!(lines.iter().any(|l| l.contains("§e§lProjets Mestier")));
        assert!(lines.iter().any(|l| l.contains("§7• §fProjet Alpha")));
    }

    #[tokio::test]
    async fn client_fetches_run_and_card_details() {
        use axum::{Json, Router, routing::get};

        let app = Router::new()
            .route(
                "/api/v1/runs/r-9",
                get(|| async {
                    Json(json!({
                        "id": "r-9",
                        "status": "succeeded",
                        "title": "Lister les DOE",
                        "card_id": "c-9",
                        "failure": null,
                        "agent": { "id": "a-1", "name": "Bernard" }
                    }))
                }),
            )
            .route(
                "/api/v1/cards/c-9",
                get(|| async {
                    Json(json!({
                        "card": { "id": "c-9" },
                        "comments": [{
                            "id": "cm-1",
                            "body": "3 DOE trouvés.",
                            "author_subject": "bernard",
                            "author_actor": "agent:bernard",
                            "created_at": "2026-11-19T10:05:00Z"
                        }],
                        "runs": []
                    }))
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        let client = CapitainerieClient::new(format!("http://{addr}"));
        let run = client.run("tok", "r-9").await.unwrap();
        assert!(run.is_terminal());
        assert!(run.is_succeeded());
        assert_eq!(run.card_id.as_deref(), Some("c-9"));

        let card = client.card("tok", "c-9").await.unwrap();
        assert_eq!(
            extract_agent_reply(&card, "bernard").as_deref(),
            Some("3 DOE trouvés.")
        );

        server.abort();
    }
}
