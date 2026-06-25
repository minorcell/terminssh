//! SSH client handler and connection logic using russh.

use std::sync::Arc;

use russh::client::{Config, Handle, Handler};
use russh::keys::PublicKey;

use crate::config::AuthMethod;

/// A minimal SSH client handler.
/// In production, `check_server_key` should verify against known_hosts.
pub struct SshClientHandler;

impl Handler for SshClientHandler {
    type Error = russh::Error;

    /// Called to verify the server's public key.
    /// V1: Accept all keys (TODO: implement known_hosts verification).
    async fn check_server_key(
        &mut self,
        _server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        // Accept all server keys for now.
        // In production, this should check against ~/.ssh/known_hosts
        // and prompt the user for unknown keys.
        Ok(true)
    }
}

/// Connect to an SSH server and authenticate.
///
/// Returns a `Handle` that can be used to open channels.
pub async fn connect_ssh(
    host: &str,
    port: u16,
    username: &str,
    auth: &AuthMethod,
) -> anyhow::Result<Handle<SshClientHandler>> {
    let config = Arc::new(Config::default());
    let addr = format!("{}:{}", host, port);

    log::info!("Connecting to {}@{}:{}", username, host, port);

    let mut handle = russh::client::connect(config, &addr, SshClientHandler).await?;

    // Authenticate based on the configured method.
    let auth_result = match auth {
        AuthMethod::Password { password } => {
            log::info!("Authenticating with password");
            handle
                .authenticate_password(username, password)
                .await?
        }
        AuthMethod::PrivateKey { key_path, passphrase } => {
            log::info!("Authenticating with private key: {}", key_path);
            let key = russh::keys::load_secret_key(key_path, passphrase.as_deref())?;

            // Determine the best RSA hash algorithm supported by the server.
            let hash_alg = handle.best_supported_rsa_hash().await?.unwrap_or(None);
            let key_with_alg = russh::keys::PrivateKeyWithHashAlg::new(
                Arc::new(key),
                hash_alg,
            );

            handle
                .authenticate_publickey(username, key_with_alg)
                .await?
        }
    };

    // Check authentication result.
    let success = match &auth_result {
        russh::client::AuthResult::Success => true,
        _ => false,
    };

    if !success {
        anyhow::bail!("Authentication failed for {}@{}", username, host);
    }

    log::info!("Authentication successful");
    Ok(handle)
}
