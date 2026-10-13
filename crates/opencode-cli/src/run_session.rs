//! Drives one `opencode run` turn through the real session engine.
//!
//! The CLI used to call `AgentExecutor::execute_streaming`, which is a single provider call that
//! never executes a tool: the run printed `[Calling tool: …]` and exited. The TUI looked fine
//! because it never exits at all. Driving `SessionPrompt` instead gives `run` the same loop the
//! server uses — tools run, awaited subagents complete, pending subtasks drain — and returns only
//! once the turn is genuinely over.

use std::io::Write;
use std::sync::{Arc, Mutex};

use opencode_provider::message::ToolDefinition;
use opencode_provider::Provider;
use opencode_session::prompt::{AgentParams, PromptInput};
use opencode_session::session::Session;
use opencode_session::{PartType, SessionPrompt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    Completed,
    /// The engine hit `MAX_STEPS` and returned `Ok(())` without telling anyone. The only way to
    /// notice is a tool call that was asked for but never answered, so that is what we look for.
    MaxStepsTruncated,
    Failed(String),
}

pub struct RunSessionDriver {
    prompt: SessionPrompt,
    pub session: Session,
    provider: Arc<dyn Provider>,
    provider_id: String,
    model_id: String,
    system_prompt: Option<String>,
    tools: Vec<ToolDefinition>,
    agent_params: AgentParams,
    sink: Arc<Mutex<Box<dyn Write + Send>>>,
    printed: Arc<Mutex<String>>,
}

impl RunSessionDriver {
    pub async fn new(
        provider: Arc<dyn Provider>,
        session: Session,
        system_prompt: Option<String>,
        agent_params: AgentParams,
    ) -> Self {
        Self::with_model(provider, session, system_prompt, agent_params, String::new(), String::new()).await
    }

    /// Without a model the session engine falls back to the literal string `default`, which every
    /// provider rejects — so `run` must carry the model the user asked for.
    pub async fn with_model(
        provider: Arc<dyn Provider>,
        session: Session,
        system_prompt: Option<String>,
        agent_params: AgentParams,
        provider_id: String,
        model_id: String,
    ) -> Self {
        Self {
            prompt: SessionPrompt::new(Arc::new(tokio::sync::RwLock::new(
                opencode_session::SessionStateManager::new(),
            ))),
            session,
            provider,
            provider_id,
            model_id,
            system_prompt,
            tools: opencode_session::default_tool_definitions().await,
            agent_params,
            sink: Arc::new(Mutex::new(Box::new(std::io::stdout()))),
            printed: Arc::new(Mutex::new(String::new())),
        }
    }

    /// Redirects transcript output, so a test can assert on what a user would see.
    pub fn with_sink(mut self, sink: Box<dyn Write + Send>) -> Self {
        self.sink = Arc::new(Mutex::new(sink));
        self
    }

    /// Replaces the toolset. Only the definitions are swapped; the driver keeps driving the real
    /// session loop, which is what these tests exist to pin.
    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = tools;
        self
    }

    pub async fn run_turn(&mut self, text: &str) -> anyhow::Result<TurnOutcome> {
        let model = if self.model_id.is_empty() {
            None
        } else {
            Some(opencode_session::prompt::ModelRef {
                provider_id: self.provider_id.clone(),
                model_id: self.model_id.clone(),
            })
        };

        let input = PromptInput {
            session_id: self.session.id.clone(),
            message_id: None,
            model,
            agent: None,
            no_reply: false,
            system: None,
            variant: None,
            parts: vec![opencode_session::PartInput::Text {
                text: text.to_string(),
            }],
            tools: None,
            permission: None,
        };

        let hook = self.transcript_hook();
        match self
            .prompt
            .prompt_with_update_hook(
                input,
                &mut self.session,
                self.provider.clone(),
                self.system_prompt.clone(),
                self.tools.clone(),
                self.agent_params.clone(),
                Some(hook),
            )
            .await
        {
            Ok(()) => Ok(classify(&self.session)),
            Err(e) => Ok(TurnOutcome::Failed(e.to_string())),
        }
    }

    /// Prints only what has not been printed yet. The hook runs on the engine task, so it must
    /// not await.
    fn transcript_hook(&self) -> opencode_session::prompt::SessionUpdateHook {
        let printed = self.printed.clone();
        let sink = self.sink.clone();
        Arc::new(move |session: &Session| {
            // The hook fires after every mutation, so the newest message is usually the user's
            // prompt or a tool result. Only the assistant's own text is transcript.
            let Some(last) = session
                .messages
                .iter()
                .rev()
                .find(|m| m.role == opencode_session::MessageRole::Assistant)
            else {
                return;
            };
            let mut text = String::new();
            for part in &last.parts {
                if let PartType::Text { text: t, .. } = &part.part_type {
                    text.push_str(t);
                }
            }
            if text.is_empty() {
                return;
            }
            let mut seen = printed.lock().unwrap_or_else(|e| e.into_inner());
            if !text.starts_with(seen.as_str()) {
                return;
            }
            let suffix = text[seen.len()..].to_string();
            seen.clear();
            seen.push_str(&text);
            drop(seen);
            if !suffix.is_empty() {
                if let Ok(mut w) = sink.lock() {
                    let _ = w.write_all(suffix.as_bytes());
                    let _ = w.flush();
                }
            }
        })
    }
}

/// Mirrors `MAX_STEPS` in `opencode-session::prompt`. When that ceiling is hit the engine logs
/// and breaks with `Ok(())`, indistinguishable from a clean finish — the only observable
/// difference is that every assistant message asked for another tool.
const ENGINE_MAX_STEPS: usize = 100;

/// A turn is truncated when it kept asking for tools right up to the engine's step ceiling.
fn classify(session: &Session) -> TurnOutcome {
    let tool_requests = session
        .messages
        .iter()
        .filter(|m| m.role == opencode_session::MessageRole::Assistant)
        .filter(|m| {
            m.parts
                .iter()
                .any(|p| matches!(p.part_type, PartType::ToolCall { .. }))
        })
        .count();

    if tool_requests >= ENGINE_MAX_STEPS {
        TurnOutcome::MaxStepsTruncated
    } else {
        TurnOutcome::Completed
    }
}