use std::{sync::Arc, thread};

use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, Sender, unbounded};
use serde_json::{Value, json};
use winit::event_loop::EventLoopProxy;

use crate::{app::UserEvent, git::models::CommitDetail};

/// Provider operation selected by the commit's published state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AiTask {
    Explain,
    Recompose,
}

/// AI provider contract.
pub(crate) trait AiProvider: Send + Sync {
    /// Requests a real response for the selected commit operation.
    fn complete(&self, task: AiTask, commit: &CommitDetail) -> Result<String>;
}

struct UnconfiguredProvider;

impl AiProvider for UnconfiguredProvider {
    fn complete(&self, _task: AiTask, _commit: &CommitDetail) -> Result<String> {
        Err(anyhow!(
            "GitKraken AI is not configured. Set KRAKEN_AI_ENDPOINT and KRAKEN_AI_API_KEY."
        ))
    }
}

struct HttpProvider {
    endpoint: String,
    api_key: String,
    model: String,
}

impl AiProvider for HttpProvider {
    fn complete(&self, task: AiTask, commit: &CommitDetail) -> Result<String> {
        let files = commit
            .files
            .iter()
            .take(80)
            .map(|file| format!("- {} ({:?})", file.path.display(), file.kind))
            .collect::<Vec<_>>()
            .join("\n");
        let (system, instruction) = match task {
            AiTask::Explain => (
                "You explain source-control changes accurately. Never invent changes absent from the supplied commit.",
                "Explain this Git commit concisely. State intent, impact, and affected components.",
            ),
            AiTask::Recompose => (
                "You write accurate conventional Git commit messages from supplied changes. Never invent changes.",
                "Propose a clearer commit subject and body. Use a past-tense conventional subject of at most 72 characters, then a concise body.",
            ),
        };
        let prompt = format!(
            "{instruction}\n\nCommit: {}\n\n{}\n\nChanged files:\n{}",
            commit.subject, commit.body, files
        );
        let payload = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": prompt}
            ]
        });
        let mut response = ureq::post(&self.endpoint)
            .header("Authorization", &format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .send_json(&payload)
            .context("call configured AI endpoint")?;
        let value: Value = response
            .body_mut()
            .read_json()
            .context("decode AI response JSON")?;
        value
            .pointer("/choices/0/message/content")
            .or_else(|| value.get("content"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("AI response did not contain message content"))
    }
}

/// Constructs the real HTTP provider only when both endpoint and key are present.
pub(crate) fn provider_from_environment() -> Arc<dyn AiProvider> {
    let endpoint = std::env::var("KRAKEN_AI_ENDPOINT").ok();
    let api_key = std::env::var("KRAKEN_AI_API_KEY").ok();
    match (endpoint, api_key) {
        (Some(endpoint), Some(api_key)) if !endpoint.is_empty() && !api_key.is_empty() => {
            Arc::new(HttpProvider {
                endpoint,
                api_key,
                model: std::env::var("KRAKEN_AI_MODEL")
                    .unwrap_or_else(|_| "claude-sonnet-4-6".to_owned()),
            })
        }
        _ => Arc::new(UnconfiguredProvider),
    }
}

/// Result of one off-thread provider call.
#[derive(Debug)]
pub(crate) struct AiEvent {
    pub(crate) result: Result<String, String>,
}

/// Serializes provider calls away from the winit event loop.
pub(crate) struct AiRunner {
    requests: Sender<(AiTask, Arc<CommitDetail>)>,
    events: Receiver<AiEvent>,
}

impl AiRunner {
    /// Starts a worker around the configured or explicit-unconfigured provider.
    pub(crate) fn new(
        provider: Arc<dyn AiProvider>,
        event_loop_proxy: Option<EventLoopProxy<UserEvent>>,
    ) -> Self {
        let (request_sender, request_receiver) = unbounded::<(AiTask, Arc<CommitDetail>)>();
        let (event_sender, event_receiver) = unbounded::<AiEvent>();
        thread::Builder::new()
            .name("kraken-ai".to_owned())
            .spawn(move || {
                while let Ok((task, commit)) = request_receiver.recv() {
                    let result = provider
                        .complete(task, &commit)
                        .map_err(|error| format!("{error:#}"));
                    if event_sender.send(AiEvent { result }).is_err() {
                        break;
                    }
                    if let Some(proxy) = &event_loop_proxy {
                        let _ = proxy.send_event(UserEvent::Ai);
                    }
                }
            })
            .expect("spawn AI provider worker");
        Self {
            requests: request_sender,
            events: event_receiver,
        }
    }

    /// Requests an explanation without blocking rendering.
    pub(crate) fn explain(&self, commit: Arc<CommitDetail>) {
        let _ = self.requests.send((AiTask::Explain, commit));
    }

    /// Requests a commit-message rewrite without blocking rendering.
    pub(crate) fn recompose(&self, commit: Arc<CommitDetail>) {
        let _ = self.requests.send((AiTask::Recompose, commit));
    }

    /// Returns the newest provider result if available.
    pub(crate) fn try_event(&self) -> Option<AiEvent> {
        self.events.try_iter().last()
    }
}
