//! Debug harness for the `OpenAI` OAuth flow.
//!
//! Binds the localhost:1455 listener exactly like the desktop app, prints
//! the authorize URL on stdout, and waits up to 15 minutes for the browser
//! callback. Used to drive the flow from outside the desktop app while
//! debugging — `cargo run --example openai_oauth_debug -p conclave-providers`.

use conclave_providers::OpenAILoginFlow;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = OpenAILoginFlow::start().await?;
    println!("AUTHORIZE_URL={}", started.url);
    println!("Waiting for callback on localhost:1455 …");
    match started
        .flow
        .wait_for_callback(std::time::Duration::from_secs(900))
        .await
    {
        Ok(tokens) => {
            let access_prefix: String = tokens.access_token.chars().take(40).collect();
            let refresh_prefix: String = tokens.refresh_token.chars().take(20).collect();
            println!("OK access_token_prefix={access_prefix}…");
            println!("OK refresh_token_prefix={refresh_prefix}…");
            println!("OK expires_at_ms={}", tokens.expires_at_ms);
        }
        Err(e) => {
            eprintln!("ERROR {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}
