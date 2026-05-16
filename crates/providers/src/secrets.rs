//! OS-native secret storage for provider API keys.
//!
//! Wraps the `keyring` crate so every key lives under one Conclave service.
//! Account naming is `provider:<id>:api_key` so collisions across providers
//! never happen.

use crate::error::ProviderError;

const SERVICE: &str = "Conclave";

fn account(provider_id: &str) -> String {
    format!("provider:{provider_id}:api_key")
}

fn map_keyring(e: keyring::Error) -> ProviderError {
    ProviderError::Other(format!("keyring: {e}"))
}

/// Persist an API key in the OS keychain for `provider_id`.
pub fn store(provider_id: &str, api_key: &str) -> Result<(), ProviderError> {
    let entry = keyring::Entry::new(SERVICE, &account(provider_id)).map_err(map_keyring)?;
    entry.set_password(api_key).map_err(map_keyring)
}

/// Look up an API key from the OS keychain. Returns `None` when absent.
pub fn load(provider_id: &str) -> Result<Option<String>, ProviderError> {
    let entry = keyring::Entry::new(SERVICE, &account(provider_id)).map_err(map_keyring)?;
    match entry.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(map_keyring(e)),
    }
}

/// Remove the API key from the OS keychain. No-ops when absent.
pub fn delete(provider_id: &str) -> Result<(), ProviderError> {
    let entry = keyring::Entry::new(SERVICE, &account(provider_id)).map_err(map_keyring)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(map_keyring(e)),
    }
}
