//! Shared LLM provider/model abstraction.
//!
//! One `ModelConfig` bundles provider + model + base url + api key. A single
//! `ProviderKind` dispatch (the only `match` on provider in the codebase)
//! builds the right rig client. Adding an OpenAI-compatible provider is a
//! one-line `PROVIDER_REGISTRY` append — no new code.

pub mod resolve;
pub mod telemetry;

use std::path::PathBuf;

use async_trait::async_trait;
use rig::agent::AgentBuilder;
use rig::client::CompletionClient;
use rig::completion::{CompletionModel, Prompt};
use rig::providers::{ollama, openai, openrouter};
use rig::tool::{Tool, ToolDyn};
use serde::Deserialize;

use std::sync::Arc;

use crate::judging::{
    AgentResponse, JudgeError, JudgeLlm, JudgeLogEvent, JudgeRunRecorder, ToolDef, log_now_ms,
};

/// Resolved LLM config — a plain bag, server-side only.
///
/// Secrets are resolved by the caller (e.g. `game-server/src/main.rs`
/// decrypts keys from `app_settings`) before construction; `build_*` never
/// reads env for the api key. No `Serialize` derive — never crosses the wire.
#[derive(Clone)]
pub struct ModelConfig {
    pub provider: String,
    /// Display name of the `llm_providers` row this came from, when it came
    /// from one at all (static fallback configs have none).
    ///
    /// `provider` above is a *registry id* — every `openai_compatible` row
    /// collapses to `"custom"` — so it cannot tell two custom endpoints
    /// apart. Telemetry and rate-limit pacing need that distinction, so the
    /// row's own name rides along.
    pub provider_name: Option<String>,
    pub model: String,
    /// Provider API base; `None` falls back to the registry default.
    pub base_url: Option<String>,
    /// Resolved api key; `None`/empty is a fail-closed error for keyed providers.
    pub api_key: Option<String>,
}

impl ModelConfig {
    /// Which *endpoint* this config talks to, for anything that must treat
    /// two separately-configured providers as separate: rate-limit buckets,
    /// logs, telemetry labels.
    ///
    /// Falls back to the registry id for static configs that came from no
    /// row. Note `provider` alone is wrong here — every `openai_compatible`
    /// row reports `"custom"`, so keying on it makes unrelated endpoints
    /// share one bucket and throttle each other.
    pub fn provider_label(&self) -> &str {
        self.provider_name.as_deref().unwrap_or(&self.provider)
    }
}

impl std::fmt::Debug for ModelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelConfig")
            .field("provider", &self.provider)
            .field("provider_name", &self.provider_name)
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

/// Transport kind. Drives the (single) client-construction `match`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    Ollama,
    OpenRouter,
    OpenAiCompatible,
}

/// Static description of a supported provider.
pub struct ProviderSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: ProviderKind,
    pub default_base_url: Option<&'static str>,
    /// Env var consulted by callers when resolving the api key (informational;
    /// `ModelConfig` itself carries the already-resolved key).
    pub api_key_env: &'static str,
    /// `app_settings` key that stores this provider's api key (e.g.
    /// `openrouter_api_key`). Empty for keyless providers like Ollama.
    pub api_key_setting: &'static str,
    /// `app_settings` key holding an admin-set base-URL override (e.g.
    /// `ollama_base_url`). Every provider has one; hosted providers rarely
    /// need it but a proxy override stays possible without code changes.
    pub base_url_setting: &'static str,
    /// Env var holding a deployment-level base-URL override (e.g.
    /// `OLLAMA_URL`). Empty when no env override applies.
    pub base_url_env: &'static str,
    pub default_model: &'static str,
}

impl ProviderSpec {
    /// Whether this provider requires an API key (fail-closed at build time).
    pub fn needs_key(&self) -> bool {
        !self.api_key_setting.is_empty()
    }

    /// Whether the admin must supply a base URL for this provider to work
    /// (no built-in default endpoint and not a hosted provider).
    pub fn needs_base_url(&self) -> bool {
        self.kind == ProviderKind::OpenAiCompatible && self.default_base_url.is_none()
    }
}

/// Base URL precedence: the row's explicit value, then the provider's
/// `base_url_env` variable, then the registry default. The env step is what
/// makes an Ollama provider saved without a base URL work inside the compose
/// stack: the daemon lives at `OLLAMA_URL` (http://ollama:11434), and the
/// registry default of localhost pointed every request at the server
/// container's own loopback — the settings-page "test provider" failed with
/// a connection error against a perfectly healthy Ollama one hostname away.
fn resolve_base_from(
    explicit: Option<String>,
    env_base: Option<String>,
    default: Option<&str>,
) -> Option<String> {
    explicit
        .filter(|s| !s.trim().is_empty())
        .or(env_base.filter(|s| !s.trim().is_empty()))
        .or_else(|| default.map(str::to_string))
}

/// Catalog of supported providers. Add an OpenAI-compatible provider here with
/// zero additional code (it reuses the `OpenAiCompatible` client arm).
pub const PROVIDER_REGISTRY: &[ProviderSpec] = &[
    ProviderSpec {
        id: "ollama",
        label: "Ollama",
        kind: ProviderKind::Ollama,
        default_base_url: Some("http://localhost:11434"),
        api_key_env: "",
        api_key_setting: "",
        base_url_setting: "ollama_base_url",
        base_url_env: "OLLAMA_URL",
        default_model: "llama3.2",
    },
    ProviderSpec {
        id: "openrouter",
        label: "OpenRouter",
        kind: ProviderKind::OpenRouter,
        default_base_url: None,
        api_key_env: "OPENROUTER_API_KEY",
        api_key_setting: "openrouter_api_key",
        base_url_setting: "openrouter_base_url",
        base_url_env: "OPENROUTER_BASE_URL",
        default_model: "",
    },
    ProviderSpec {
        id: "opencode",
        label: "OpenCode Zen",
        kind: ProviderKind::OpenAiCompatible,
        default_base_url: Some("https://opencode.ai/zen/v1"),
        api_key_env: "OPENCODE_API_KEY",
        api_key_setting: "opencode_api_key",
        base_url_setting: "opencode_base_url",
        base_url_env: "OPENCODE_BASE_URL",
        default_model: "",
    },
    // Escape hatch: any OpenAI-compatible endpoint, configured entirely from
    // admin settings (base URL + key + model) — adding a brand-new provider
    // needs no code change anywhere.
    ProviderSpec {
        id: "custom",
        label: "Custom (OpenAI-compatible)",
        kind: ProviderKind::OpenAiCompatible,
        default_base_url: None,
        api_key_env: "CUSTOM_LLM_API_KEY",
        api_key_setting: "custom_api_key",
        base_url_setting: "custom_base_url",
        base_url_env: "CUSTOM_LLM_BASE_URL",
        default_model: "",
    },
];

/// Lookup a provider spec by id (the registry is the source of truth).
pub fn lookup_provider(id: &str) -> Option<&'static ProviderSpec> {
    PROVIDER_REGISTRY.iter().find(|p| p.id == id)
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("unknown provider: {0}")]
    UnknownProvider(String),
    #[error("api key required for provider {0}")]
    MissingApiKey(String),
    #[error("base url required for provider {0}")]
    MissingBaseUrl(String),
    #[error("client build failed: {0}")]
    ClientBuild(String),
    #[error(transparent)]
    Judge(#[from] JudgeError),
}

impl ModelConfig {
    fn resolve_spec(&self) -> Result<&'static ProviderSpec, LlmError> {
        lookup_provider(&self.provider)
            .ok_or_else(|| LlmError::UnknownProvider(self.provider.clone()))
    }

    fn resolve_base(&self, spec: &ProviderSpec) -> Option<String> {
        let env_base = if spec.base_url_env.is_empty() {
            None
        } else {
            std::env::var(spec.base_url_env).ok()
        };
        resolve_base_from(self.base_url.clone(), env_base, spec.default_base_url)
    }

    /// Build a `JudgeLlm` impl (tool-calling) for this config. The returned
    /// boxed value builds a rig Agent with tools each turn; `repo_dir` and
    /// `task_commit_sha` are captured for the git-read tools.
    pub fn build_judge(
        &self,
        repo_dir: PathBuf,
        task_commit_sha: Option<String>,
        task_stats_json: Option<String>,
    ) -> Result<Box<dyn JudgeLlm>, LlmError> {
        self.build_judge_recorded(repo_dir, task_commit_sha, task_stats_json, None)
    }

    /// Like [`ModelConfig::build_judge`], with a shared [`JudgeRunRecorder`]
    /// that captures every LLM turn and tool call (rig runs tools
    /// internally, so the caller cannot observe them otherwise).
    pub fn build_judge_recorded(
        &self,
        repo_dir: PathBuf,
        task_commit_sha: Option<String>,
        task_stats_json: Option<String>,
        recorder: Option<Arc<JudgeRunRecorder>>,
    ) -> Result<Box<dyn JudgeLlm>, LlmError> {
        self.build_judge_full(repo_dir, task_commit_sha, task_stats_json, recorder, None)
    }

    /// Like [`ModelConfig::build_judge_recorded`], with a live
    /// [`ProbeRegistrar`] so a rig-run judge can register probes mid-run.
    pub fn build_judge_full(
        &self,
        repo_dir: PathBuf,
        task_commit_sha: Option<String>,
        task_stats_json: Option<String>,
        recorder: Option<Arc<JudgeRunRecorder>>,
        registrar: Option<Arc<dyn crate::judging::ProbeRegistrar>>,
    ) -> Result<Box<dyn JudgeLlm>, LlmError> {
        let spec = self.resolve_spec()?;
        let base = self.resolve_base(spec);
        let model = self.model.clone();
        // opencode's OpenAI passthrough (gpt-5.6-*) applies a server-side
        // default reasoning effort and then refuses function tools with it
        // on /v1/chat/completions ("set reasoning_effort to 'none'"). Judges
        // always carry tools, so send exactly that for these models.
        let additional_params = if model.starts_with("gpt-5.6") {
            Some(serde_json::json!({ "reasoning_effort": "none" }))
        } else {
            None
        };

        let boxed: Box<dyn JudgeLlm> = match spec.kind {
            ProviderKind::Ollama => {
                let base = base.unwrap_or_else(|| "http://localhost:11434".to_string());
                // The key is optional here, unlike the hosted providers below.
                // A local daemon needs none — signed in, it serves the `-cloud`
                // models by proxying to ollama.com itself — but a secured or
                // hosted Ollama endpoint requires a Bearer token. Hardcoding
                // the default (empty) key made that second case impossible:
                // the row's key was resolved and then dropped, so the request
                // went out unauthenticated. `OllamaApiKey::from` maps an empty
                // string back to "no auth", so both cases share one path.
                let key = self.api_key.clone().unwrap_or_default();
                let client = ollama::Client::builder()
                    .api_key(ollama::OllamaApiKey::from(key))
                    .base_url(&base)
                    .build()
                    .map_err(|e| LlmError::ClientBuild(e.to_string()))?;
                Box::new(RigJudge {
                    additional_params: additional_params.clone(),
                    model: client.completion_model(model.clone()),
                    model_name: model,
                    repo_dir,
                    task_commit_sha,
                    task_stats_json,
                    recorder,
                    registrar: registrar.clone(),
                })
            }
            ProviderKind::OpenRouter => {
                let key = self
                    .api_key
                    .clone()
                    .ok_or_else(|| LlmError::MissingApiKey(self.provider.clone()))?;
                let client = match &base {
                    Some(b) => openrouter::Client::builder()
                        .api_key(key)
                        .base_url(b)
                        .build()
                        .map_err(|e| LlmError::ClientBuild(e.to_string()))?,
                    None => openrouter::Client::new(key)
                        .map_err(|e| LlmError::ClientBuild(e.to_string()))?,
                };
                Box::new(RigJudge {
                    additional_params: additional_params.clone(),
                    model: client.completion_model(model.clone()),
                    model_name: model,
                    repo_dir,
                    task_commit_sha,
                    task_stats_json,
                    recorder,
                    registrar: registrar.clone(),
                })
            }
            ProviderKind::OpenAiCompatible => {
                let key = self
                    .api_key
                    .clone()
                    .ok_or_else(|| LlmError::MissingApiKey(self.provider.clone()))?;
                let base = base.ok_or_else(|| LlmError::MissingBaseUrl(self.provider.clone()))?;
                let client = openai::CompletionsClient::builder()
                    .api_key(&key)
                    .base_url(&base)
                    .build()
                    .map_err(|e| LlmError::ClientBuild(e.to_string()))?;
                Box::new(RigJudge {
                    additional_params: additional_params.clone(),
                    model: client.completion_model(model.clone()),
                    model_name: model,
                    repo_dir,
                    task_commit_sha,
                    task_stats_json,
                    recorder,
                    registrar: registrar.clone(),
                })
            }
        };
        Ok(boxed)
    }

    /// Plain completion (no tools) — used by the `server` crate. Reuses the
    /// judge path with an empty tool set, so there is exactly one provider
    /// `match` in the codebase.
    pub async fn complete(&self, system: &str, user: &str) -> Result<String, LlmError> {
        let judge = self.build_judge(PathBuf::new(), None, None)?;
        match judge.run_agent(system, user, vec![], None).await? {
            AgentResponse::Final { text } => Ok(text),
            AgentResponse::ToolCall { .. } => Err(LlmError::ClientBuild(
                "unexpected tool call in plain completion".into(),
            )),
        }
    }
}

/// Telemetry size caps: prompt/completion text, per-tool result, and the
/// serialized turn transcript. Keeps `judge_results.run_log` bounded while
/// still carrying full-fidelity content for typical runs.
const LLM_TEXT_CAP: usize = 16_000;
const TOOL_OUTPUT_CAP: usize = 4_000;
const TRANSCRIPT_CAP: usize = 64_000;

/// Generic rig-backed `JudgeLlm`. `M` is the provider's concrete completion
/// model type; `AgentBuilder` is uniform across providers, so the agent/tool
/// loop lives here once.
struct RigJudge<M> {
    /// Extra provider params merged into every completion request (e.g.
    /// `reasoning_effort: "none"` where an endpoint would otherwise refuse
    /// function tools). `None` for providers that need nothing special.
    additional_params: Option<serde_json::Value>,
    model: M,
    /// Requested model id, recorded on telemetry events
    /// (gen_ai.request.model).
    model_name: String,
    repo_dir: PathBuf,
    task_commit_sha: Option<String>,
    task_stats_json: Option<String>,
    recorder: Option<Arc<JudgeRunRecorder>>,
    registrar: Option<Arc<dyn crate::judging::ProbeRegistrar>>,
}

#[async_trait]
impl<M> JudgeLlm for RigJudge<M>
where
    M: CompletionModel + Clone + Send + Sync + 'static,
{
    async fn run_agent(
        &self,
        system: &str,
        user: &str,
        tools: Vec<ToolDef>,
        prior_tool_result: Option<&str>,
    ) -> Result<AgentResponse, JudgeError> {
        self.run_agent_with_images(system, user, tools, prior_tool_result, &[])
            .await
    }

    async fn run_agent_with_images(
        &self,
        system: &str,
        user: &str,
        tools: Vec<ToolDef>,
        prior_tool_result: Option<&str>,
        images: &[crate::judging::JudgeImage],
    ) -> Result<AgentResponse, JudgeError> {
        let boxed: Vec<Box<dyn ToolDyn>> = tools
            .iter()
            .map(|td| {
                Box::new(JudgeTool {
                    def: td.clone(),
                    repo_dir: self.repo_dir.clone(),
                    task_commit_sha: self.task_commit_sha.clone(),
                    task_stats_json: self.task_stats_json.clone(),
                    recorder: self.recorder.clone(),
                    registrar: self.registrar.clone(),
                }) as Box<dyn ToolDyn>
            })
            .collect();

        let prompt = if let Some(result) = prior_tool_result {
            format!("{user}\n\nTool result:\n{result}")
        } else {
            user.to_string()
        };

        // Text plus any attached screenshots as one multimodal user turn.
        // Providers that cannot take images reject the request, which the
        // ordinary retry/failover path handles like any provider error.
        let mut parts: Vec<rig::completion::message::UserContent> =
            vec![rig::completion::message::UserContent::text(prompt.clone())];
        for image in images {
            let media_type = match image.media_type.as_str() {
                "image/png" => Some(rig::completion::message::ImageMediaType::PNG),
                "image/jpeg" | "image/jpg" => Some(rig::completion::message::ImageMediaType::JPEG),
                "image/gif" => Some(rig::completion::message::ImageMediaType::GIF),
                "image/webp" => Some(rig::completion::message::ImageMediaType::WEBP),
                _ => None,
            };
            parts.push(rig::completion::message::UserContent::image_base64(
                image.base64.clone(),
                media_type,
                // OpenAI-compatible conversion REJECTS an image without a
                // detail level ("OpenAI image URI must have image detail").
                // High: these are UI screenshots, and the judge's whole job
                // is reading small text and spacing off them.
                Some(rig::completion::message::ImageDetail::High),
            ));
        }
        let message = rig::completion::message::Message::User {
            content: rig::OneOrMany::many(parts).unwrap_or_else(|_| {
                rig::OneOrMany::one(rig::completion::message::UserContent::text(prompt.clone()))
            }),
        };

        let started_at = log_now_ms();
        let started = std::time::Instant::now();
        let mut builder = AgentBuilder::new(self.model.clone())
            .preamble(system)
            .tools(boxed);
        if let Some(params) = &self.additional_params {
            builder = builder.additional_params(params.clone());
        }
        let sent = builder
            .build()
            .prompt(message)
            .max_turns(crate::judging::MAX_TOOL_CALLS)
            .extended_details()
            .await;
        let duration_ms = started.elapsed().as_millis() as i64;
        // gen_ai.prompt equivalent: system instructions + user prompt.
        let telemetry_input = || {
            Some(crate::judging::truncate_chars(
                &format!("[system]\n{system}\n\n[user]\n{prompt}"),
                LLM_TEXT_CAP,
            ))
        };
        let resp = match sent {
            Ok(r) => {
                if let Some(rec) = &self.recorder {
                    // Full turn transcript (assistant messages incl. tool
                    // calls + tool results), size-capped.
                    let messages = r.messages.as_ref().and_then(|m| {
                        let v = serde_json::to_value(m).ok()?;
                        (v.to_string().chars().count() <= TRANSCRIPT_CAP).then_some(v)
                    });
                    rec.record(JudgeLogEvent {
                        at_ms: started_at,
                        kind: "llm".to_string(),
                        name: None,
                        args: None,
                        output_chars: Some(r.output.chars().count() as u64),
                        duration_ms,
                        tokens_input: Some(r.usage.input_tokens),
                        tokens_output: Some(r.usage.output_tokens),
                        tokens_cache_read: Some(r.usage.cached_input_tokens),
                        tokens_cache_write: Some(r.usage.cache_creation_input_tokens),
                        model: Some(self.model_name.clone()),
                        input: telemetry_input(),
                        output: Some(crate::judging::truncate_chars(&r.output, LLM_TEXT_CAP)),
                        messages,
                        error: None,
                    });
                }
                r.output
            }
            Err(e) => {
                if let Some(rec) = &self.recorder {
                    rec.record(JudgeLogEvent {
                        at_ms: started_at,
                        kind: "llm".to_string(),
                        name: None,
                        args: None,
                        output_chars: None,
                        duration_ms,
                        tokens_input: None,
                        tokens_output: None,
                        tokens_cache_read: None,
                        tokens_cache_write: None,
                        model: Some(self.model_name.clone()),
                        input: telemetry_input(),
                        output: None,
                        messages: None,
                        error: Some(e.to_string()),
                    });
                }
                return Err(JudgeError::Llm(format!("prompt: {e}")));
            }
        };

        // rig auto-runs tools to completion; `resp` is the final text. If it
        // parses as a JSON verdict shape, treat as a tool call so `run_judge`
        // can re-dispatch. Simple heuristic; refine if needed.
        if let Ok(call) = serde_json::from_str::<ToolCallShape>(&resp)
            && !call.tool.is_empty()
        {
            return Ok(AgentResponse::ToolCall {
                name: call.tool,
                args: call.input,
            });
        }
        Ok(AgentResponse::Final { text: resp })
    }
}

/// LLM-emit shape mirroring Ollama's tool-call response format.
#[derive(Debug, Deserialize)]
struct ToolCallShape {
    tool: String,
    input: serde_json::Value,
}

/// rig-core `Tool` wrapping one arena-core judge tool def. Dispatches to
/// `arena_core::judging::dispatch_tool`.
struct JudgeTool {
    def: ToolDef,
    repo_dir: PathBuf,
    task_commit_sha: Option<String>,
    task_stats_json: Option<String>,
    recorder: Option<Arc<JudgeRunRecorder>>,
    registrar: Option<Arc<dyn crate::judging::ProbeRegistrar>>,
}

impl Tool for JudgeTool {
    const NAME: &'static str = "judge_tool";
    type Error = std::convert::Infallible;
    type Args = serde_json::Value;
    type Output = String;

    fn name(&self) -> String {
        self.def.name.clone()
    }

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: self.def.name.clone(),
            description: self.def.description.clone(),
            parameters: self.def.params.clone(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let started_at = log_now_ms();
        let started = std::time::Instant::now();
        let out = match (&self.def.name[..], &self.registrar) {
            ("register_probe", Some(reg)) => reg.register(&args).await,
            _ => {
                crate::judging::dispatch_tool(
                    &self.repo_dir,
                    &self.def.name,
                    &args,
                    self.task_commit_sha.as_deref(),
                    self.task_stats_json.as_deref(),
                    &self.def.scope,
                )
                .await
            }
        };
        if let Some(rec) = &self.recorder {
            let mut args_str = args.to_string();
            if args_str.chars().count() > 500 {
                args_str = args_str.chars().take(500).collect::<String>() + "\u{2026}";
            }
            rec.record(JudgeLogEvent {
                at_ms: started_at,
                kind: "tool".to_string(),
                name: Some(self.def.name.clone()),
                args: Some(args_str),
                output_chars: Some(out.chars().count() as u64),
                duration_ms: started.elapsed().as_millis() as i64,
                tokens_input: None,
                tokens_output: None,
                tokens_cache_read: None,
                tokens_cache_write: None,
                model: None,
                input: None,
                output: Some(crate::judging::truncate_chars(&out, TOOL_OUTPUT_CAP)),
                messages: None,
                error: None,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{lookup_provider, resolve_base_from};

    #[test]
    fn registry_covers_expected_providers() {
        for id in ["ollama", "openrouter", "opencode", "custom"] {
            assert!(
                lookup_provider(id).is_some(),
                "missing registry entry: {id}"
            );
        }
        assert!(lookup_provider("nope").is_none());
    }

    #[test]
    fn custom_provider_requires_key_and_base_url() {
        let spec = lookup_provider("custom").unwrap();
        assert!(spec.needs_key());
        assert!(spec.needs_base_url());
        let ollama = lookup_provider("ollama").unwrap();
        assert!(!ollama.needs_key());
        assert!(!ollama.needs_base_url());
    }

    #[test]
    fn base_url_precedence_is_explicit_then_env_then_default() {
        let default = Some("http://localhost:11434");
        // An admin-entered URL always wins.
        assert_eq!(
            resolve_base_from(
                Some("http://me:1".into()),
                Some("http://env:2".into()),
                default
            )
            .as_deref(),
            Some("http://me:1")
        );
        // No explicit URL: the environment names where the daemon actually
        // lives (OLLAMA_URL in the compose stack), not the registry default.
        assert_eq!(
            resolve_base_from(None, Some("http://ollama:11434".into()), default).as_deref(),
            Some("http://ollama:11434")
        );
        // Blank strings do not shadow the fallbacks.
        assert_eq!(
            resolve_base_from(Some("  ".into()), Some("".into()), default).as_deref(),
            Some("http://localhost:11434")
        );
        assert_eq!(resolve_base_from(None, None, None), None);
    }
}
