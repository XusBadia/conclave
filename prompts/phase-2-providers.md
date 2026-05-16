# Phase 2 — Provider Layer

Pre-requisite: Phase 1 complete, ingest + search working.

---

Phase 1 done. Now Phase 2: the LLM Provider layer. Re-read
`ARCHITECTURE.md` (the `providers` crate section) and `PLAN.md` (Phase 2).
Implement to spec.

## What to build

A unified, swappable inference layer with four providers: Anthropic API,
OpenAI API, OpenRouter API, and Ollama local. OAuth-based providers come
in an optional Phase 2.5.

### 1. The trait

In `crates/providers/src/lib.rs`:

```rust
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> ProviderCapabilities;
    fn requires_network(&self) -> bool;

    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError>;
}
```

Types:

- `ProviderCapabilities { max_context_tokens, supports_json_mode,
  supports_streaming, vision: bool }`
- `CompletionRequest { model: String, messages: Vec<Message>,
  max_output_tokens: Option<u32>, temperature: Option<f32>,
  json_schema: Option<serde_json::Value> }`
- `Message { role: MessageRole, content: String }` with role
  `System | User | Assistant`.
- `CompletionResponse { text: String, usage: Usage, model: String }`
- `Usage { input_tokens, output_tokens }`
- `ProviderError` with `thiserror` variants: `Network`, `Auth`,
  `RateLimit`, `BadRequest(String)`, `ContextOverflow`, `Other(String)`.

### 2. Implementations

Each in its own module, feature-gated:

#### `anthropic_api` (feature `provider-anthropic`)

- POST `https://api.anthropic.com/v1/messages`.
- Header: `x-api-key`, `anthropic-version: 2023-06-01`,
  `content-type: application/json`.
- Map our `CompletionRequest` to the Anthropic Messages shape (system
  prompt extracted from messages list, rest as `user/assistant`).
- Default model: `claude-sonnet-4-6-20250929` (current Sonnet; user can
  override).
- Parse usage from response.

#### `openai_api` (feature `provider-openai`)

- POST `https://api.openai.com/v1/chat/completions`.
- Standard chat completions schema.
- Header: `Authorization: Bearer <key>`.
- Default model: `gpt-5` if exists else fall back to whatever the user
  configured.
- JSON mode: pass `response_format: { type: "json_object" }` when
  `json_schema` is set (don't try structured outputs yet; flat JSON
  mode is enough for now).

#### `openrouter_api` (feature `provider-openrouter`)

- POST `https://openrouter.ai/api/v1/chat/completions`.
- Same shape as OpenAI chat completions.
- Header: `Authorization: Bearer <key>`, plus `HTTP-Referer` and
  `X-Title: Conclave`.
- Default model: configurable; no hard default.

#### `ollama_local` (feature `provider-ollama`, on by default)

- POST `http://<host>:<port>/api/chat` (default
  `http://localhost:11434`).
- Detect availability by hitting `/api/tags` on startup; if it 404s or
  refuses connection, the provider self-reports unavailable.
- No auth header.

### 3. Secret storage

- Use the `keyring` crate.
- Service name: `Conclave`.
- Key name: `provider:<id>:api_key` (e.g., `provider:anthropic:api_key`).
- Never write keys to TOML config files.
- CLI: `providers set <id>` prompts for the key (hidden input via
  `rpassword`), stores it, then runs a "hello" test.

### 4. Provider registry

In `crates/providers/src/registry.rs`:

- `ProviderRegistry` holds the configured + available providers.
- Loads from config + keyring on startup.
- Exposes `get(id) -> Option<&dyn LlmProvider>`.
- Exposes `default_for(task: TaskKind)` where `TaskKind` is `Light` or
  `Reasoning`. Mapping comes from config.

### 5. CLI subcommands

- `conclave-cli providers list` — shows id, configured (y/n), available
  (y/n), default model, requires network.
- `conclave-cli providers set <id>` — interactive setup.
- `conclave-cli providers test <id> [--prompt "hi"]` — runs a
  completion, prints latency and tokens.
- `conclave-cli providers remove <id>` — clears key from keyring +
  config.
- `conclave-cli config set routing.light <provider-id:model>` and
  `routing.reasoning <provider-id:model>`.

### 6. Mock provider for tests

Add a `mock` provider behind `cfg(test)` that returns canned responses.
Used in integration tests so we never hit the network in CI.

### 7. Tests

- Unit test for each provider's request mapping (input → expected JSON
  body). Use `wiremock` to fake the HTTP server.
- Auth error returns `ProviderError::Auth`, not `Network`.
- Rate limit (429) returns `ProviderError::RateLimit`.
- Ollama unavailable returns gracefully, doesn't panic.
- Registry test: switching default provider takes effect on next call.

## Quality bar

- Same as previous phases. Plus: no provider impl may panic on any HTTP
  response shape; all errors typed.
- Secrets must not appear in logs at any tracing level. Add a regex
  smoke test on log output to assert this.

## How to work

- Trait + registry + types first. Then each provider impl as a separate
  commit. Then tests. Then CLI wiring.
- Tracking issue: "Phase 2 — Provider Layer".

When `providers test` works for at least one API-key provider end-to-end
and Ollama detection works, we move on. Phase 2.5 (OAuth providers) is
optional and only worth doing once Phase 4 reveals a real need.
