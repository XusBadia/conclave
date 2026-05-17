//! `conclave-cli providers` — inspect, configure and test LLM providers.

use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Subcommand};

use conclave_providers::{
    open_in_browser, persist_tokens, secrets, AnthropicLoginFlow, AnthropicOAuthProvider,
    AnthropicProvider, AppleIntelligenceAvailability, AppleIntelligenceProvider, CompletionRequest,
    LlmProvider, Message, MockProvider, OllamaProvider, OpenAILoginFlow, OpenAIOAuthProvider,
    OpenAiProvider, OpenRouterProvider, APPLE_INTELLIGENCE_MODEL_LABEL, KNOWN_PROVIDERS,
    OAUTH_PROVIDERS,
};

use super::CommandContext;

/// Arguments for the `providers` subcommand.
#[derive(Debug, Args)]
pub(crate) struct ProvidersArgs {
    #[command(subcommand)]
    pub(crate) action: ProvidersAction,
}

/// Sub-actions exposed by `providers`.
#[derive(Debug, Subcommand)]
pub(crate) enum ProvidersAction {
    /// List every known provider and whether it is configured.
    List,
    /// Store an API key in the OS keychain and run a hello call.
    Set {
        /// Provider id (`anthropic`, `openai`, `openrouter`, …). For
        /// `ollama` no key is needed; running `set` just verifies the
        /// local server.
        id: String,
    },
    /// Sign in with the user's Claude Max / ChatGPT subscription via
    /// browser-based OAuth (PKCE).
    Login {
        /// Provider id (`anthropic-oauth`, `openai-oauth`).
        id: String,
    },
    /// Forget the OAuth tokens stored for an OAuth provider.
    Logout {
        /// Provider id.
        id: String,
    },
    /// Run a one-shot completion against a configured provider.
    Test {
        /// Provider id.
        id: String,
        /// Prompt to send. Defaults to "hi".
        #[arg(long, default_value = "hi")]
        prompt: String,
        /// Optional explicit model id (otherwise the provider default).
        #[arg(long)]
        model: Option<String>,
    },
    /// Remove a stored API key from the OS keychain.
    Remove {
        /// Provider id.
        id: String,
    },
}

/// Execute the `providers` subcommand.
pub(crate) async fn run(ctx: &CommandContext, args: ProvidersArgs) -> Result<()> {
    match args.action {
        ProvidersAction::List => list(ctx).await,
        ProvidersAction::Set { id } => set(&id).await,
        ProvidersAction::Login { id } => login(ctx, &id).await,
        ProvidersAction::Logout { id } => logout(ctx, &id),
        ProvidersAction::Test { id, prompt, model } => test(ctx, &id, prompt, model).await,
        ProvidersAction::Remove { id } => remove(&id),
    }
}

async fn login(ctx: &CommandContext, id: &str) -> Result<()> {
    let config_dir = ctx.paths.config_dir().to_path_buf();
    match id {
        "anthropic-oauth" => {
            let started = AnthropicLoginFlow::start()?;
            println!("Opening your browser to sign in with your Anthropic account…");
            println!(
                "If it does not open, paste this URL into a browser:\n  {}\n",
                started.url
            );
            let _ = open_in_browser(&started.url);
            println!(
                "After signing in, Anthropic will show a one-time code. Copy + paste \
                 it below (format `<code>#<state>`):"
            );
            let code = rpassword::prompt_password("code: ")
                .context("could not read code from terminal")?;
            let tokens = started.flow.complete(&code).await?;
            persist_tokens(&config_dir, "anthropic-oauth", &tokens)?;
            println!(
                "✓ Anthropic OAuth tokens stored in {}",
                config_dir.display()
            );
        }
        "openai-oauth" => {
            let started = OpenAILoginFlow::start().await?;
            println!("Opening your browser to sign in with your ChatGPT account…");
            println!(
                "If it does not open, paste this URL into a browser:\n  {}\n",
                started.url
            );
            let _ = open_in_browser(&started.url);
            println!("Waiting for the redirect (timeout: 5 minutes)…");
            let tokens = started
                .flow
                .wait_for_callback(std::time::Duration::from_secs(300))
                .await?;
            persist_tokens(&config_dir, "openai-oauth", &tokens)?;
            println!("✓ OpenAI OAuth tokens stored in {}", config_dir.display());
        }
        other => anyhow::bail!(
            "unknown OAuth provider `{other}` — supported: {}",
            OAUTH_PROVIDERS.join(", ")
        ),
    }
    Ok(())
}

fn logout(ctx: &CommandContext, id: &str) -> Result<()> {
    let path = ctx
        .paths
        .config_dir()
        .join("oauth")
        .join(format!("{id}.json"));
    match std::fs::remove_file(&path) {
        Ok(()) => println!("removed {}", path.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("nothing to remove at {}", path.display());
        }
        Err(e) => anyhow::bail!("could not remove {}: {e}", path.display()),
    }
    Ok(())
}

async fn list(ctx: &CommandContext) -> Result<()> {
    let default = ctx.config.providers.default.as_deref().unwrap_or("<unset>");
    println!("providers — default: {default}\n");
    println!("id                  configured   available    auth        default-model");
    for id in KNOWN_PROVIDERS {
        // Apple Intelligence is omitted on hosts where it isn't
        // user-actionable, mirroring `list_providers` in the Tauri
        // backend so the CLI and the desktop UI agree on what's
        // visible.
        if *id == "apple-intelligence" {
            let avail = AppleIntelligenceProvider::new().availability().await;
            if !avail.is_user_actionable() {
                continue;
            }
            let available = matches!(avail, AppleIntelligenceAvailability::Available);
            println!(
                "{id:<19} {:<12} {:<12} {:<11} {}",
                yes_no(available),
                yes_no(available),
                "local",
                APPLE_INTELLIGENCE_MODEL_LABEL,
            );
            continue;
        }
        let configured = secrets::load(id).unwrap_or(None).is_some();
        let (available, default_model) = match *id {
            "ollama" => {
                let p = OllamaProvider::new();
                let alive = p.ping().await;
                (alive, "llama3.1:8b".to_string())
            }
            "anthropic" => (configured, "claude-sonnet-4-6-20250929".to_string()),
            "openai" => (configured, "gpt-5".to_string()),
            "openrouter" => (configured, "<set per call>".to_string()),
            _ => (false, "-".to_string()),
        };
        let auth = if *id == "ollama" { "local" } else { "api-key" };
        println!(
            "{id:<19} {:<12} {:<12} {auth:<11} {default_model}",
            yes_no(configured),
            yes_no(available),
        );
    }
    println!("\n— OAuth providers (experimental) —");
    for id in OAUTH_PROVIDERS {
        let (configured, available, default_model, label) = match *id {
            "anthropic-oauth" => match AnthropicOAuthProvider::from_default_location() {
                Ok(p) => (
                    true,
                    true,
                    "claude-sonnet-4-6-20250929".to_string(),
                    p.subscription_type().unwrap_or_else(|| "claude".into()),
                ),
                Err(_) => (false, false, "—".into(), "run `claude login`".into()),
            },
            "openai-oauth" => match OpenAIOAuthProvider::from_default_location() {
                Ok(p) => (
                    true,
                    true,
                    "gpt-5".to_string(),
                    p.account_id().unwrap_or_else(|| "chatgpt".into()),
                ),
                Err(_) => (false, false, "—".into(), "run `codex login`".into()),
            },
            _ => (false, false, "—".into(), "—".into()),
        };
        println!(
            "{id:<19} {:<12} {:<12} {:<11} {default_model}",
            yes_no(configured),
            yes_no(available),
            label
        );
    }
    Ok(())
}

async fn set(id: &str) -> Result<()> {
    if id == "ollama" {
        let p = OllamaProvider::new();
        if p.ping().await {
            println!("ollama: server reachable at http://localhost:11434");
        } else {
            anyhow::bail!(
                "ollama: no response on http://localhost:11434 — start it with `ollama serve`"
            );
        }
        return Ok(());
    }
    if id == "anthropic-oauth" {
        match AnthropicOAuthProvider::from_default_location() {
            Ok(p) => {
                println!(
                    "anthropic-oauth: credentials detected (subscription: {})",
                    p.subscription_type().as_deref().unwrap_or("claude")
                );
            }
            Err(e) => anyhow::bail!("{e}\nRun `claude login` in a terminal first."),
        }
        return Ok(());
    }
    if id == "openai-oauth" {
        match OpenAIOAuthProvider::from_default_location() {
            Ok(p) => {
                println!(
                    "openai-oauth: credentials detected (account: {})",
                    p.account_id().as_deref().unwrap_or("chatgpt")
                );
            }
            Err(e) => anyhow::bail!("{e}\nRun `codex login` in a terminal first."),
        }
        return Ok(());
    }
    if !KNOWN_PROVIDERS.contains(&id) {
        anyhow::bail!(
            "unknown provider `{id}` — known: {} · oauth: {}",
            KNOWN_PROVIDERS.join(", "),
            OAUTH_PROVIDERS.join(", "),
        );
    }
    let api_key = rpassword::prompt_password(format!("API key for {id}: "))
        .context("could not read API key from terminal")?;
    if api_key.trim().is_empty() {
        anyhow::bail!("API key cannot be empty");
    }
    secrets::store(id, &api_key)
        .with_context(|| format!("could not store API key for {id} in keychain"))?;
    println!("stored API key for {id} in keychain");

    println!("verifying with a hello call…");
    let provider = build_provider(id, &api_key, None)?;
    let resp = provider
        .complete(CompletionRequest {
            model: String::new(),
            messages: vec![Message::user("Reply with one word: hello.")],
            max_output_tokens: Some(20),
            temperature: Some(0.0),
            json_schema: None,
            allow_web_search: false,
            images: Vec::new(),
        })
        .await
        .with_context(|| format!("hello call to {id} failed"))?;
    println!(
        "ok — model={} usage={}+{} tokens",
        resp.model, resp.usage.input_tokens, resp.usage.output_tokens
    );
    Ok(())
}

async fn test(ctx: &CommandContext, id: &str, prompt: String, model: Option<String>) -> Result<()> {
    let api_key = match id {
        "ollama" | "apple-intelligence" | "anthropic-oauth" | "openai-oauth" => String::new(),
        _ => secrets::load(id)?
            .ok_or_else(|| anyhow!("no API key configured for {id} — run `providers set {id}`"))?,
    };
    let provider = build_provider_with_ctx(ctx, id, &api_key, model.clone())?;

    let start = Instant::now();
    let resp = provider
        .complete(CompletionRequest {
            model: model.unwrap_or_default(),
            messages: vec![Message::user(prompt)],
            max_output_tokens: Some(256),
            temperature: Some(0.2),
            json_schema: None,
            allow_web_search: false,
            images: Vec::new(),
        })
        .await
        .with_context(|| format!("completion against {id} failed"))?;
    let elapsed = start.elapsed();

    println!("model:   {}", resp.model);
    println!("latency: {elapsed:.2?}");
    println!(
        "tokens:  in={} out={}",
        resp.usage.input_tokens, resp.usage.output_tokens
    );
    println!("\n--- response ---");
    println!("{}", resp.text);
    Ok(())
}

fn remove(id: &str) -> Result<()> {
    secrets::delete(id).with_context(|| format!("could not remove API key for {id}"))?;
    println!("removed API key for {id} from keychain");
    Ok(())
}

fn build_provider_with_ctx(
    ctx: &CommandContext,
    id: &str,
    api_key: &str,
    model: Option<String>,
) -> Result<Box<dyn LlmProvider>> {
    let config_dir = ctx.paths.config_dir();
    match id {
        "anthropic-oauth" => {
            let mut p = AnthropicOAuthProvider::from_conclave_or_cli(config_dir)
                .map_err(|e| anyhow!("{e}"))?;
            if let Some(m) = model {
                p = p.with_model(m);
            }
            return Ok(Box::new(p));
        }
        "openai-oauth" => {
            let mut p = OpenAIOAuthProvider::from_conclave_or_cli(config_dir)
                .map_err(|e| anyhow!("{e}"))?;
            if let Some(m) = model {
                p = p.with_model(m);
            }
            return Ok(Box::new(p));
        }
        _ => {}
    }
    build_provider(id, api_key, model)
}

fn build_provider(id: &str, api_key: &str, model: Option<String>) -> Result<Box<dyn LlmProvider>> {
    match id {
        "anthropic" => {
            let mut p = AnthropicProvider::new(api_key.to_owned());
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Ok(Box::new(p))
        }
        "openai" => {
            let mut p = OpenAiProvider::new(api_key.to_owned());
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Ok(Box::new(p))
        }
        "openrouter" => {
            let mut p = OpenRouterProvider::new(api_key.to_owned());
            if let Some(m) = model {
                p = p.with_model(m);
            } else {
                p = p.with_model("anthropic/claude-3.5-sonnet");
            }
            Ok(Box::new(p))
        }
        "ollama" => {
            let mut p = OllamaProvider::new();
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Ok(Box::new(p))
        }
        "apple-intelligence" => Ok(Box::new(AppleIntelligenceProvider::new())),
        "anthropic-oauth" => {
            let mut p =
                AnthropicOAuthProvider::from_default_location().map_err(|e| anyhow!("{e}"))?;
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Ok(Box::new(p))
        }
        "openai-oauth" => {
            let mut p = OpenAIOAuthProvider::from_default_location().map_err(|e| anyhow!("{e}"))?;
            if let Some(m) = model {
                p = p.with_model(m);
            }
            Ok(Box::new(p))
        }
        "mock" => Ok(Box::new(MockProvider::with_response("mock response"))),
        other => Err(anyhow!("unknown provider `{other}`")),
    }
}

const fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}
