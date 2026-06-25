use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// SSH authentication method.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum AuthMethod {
    Password { password: String },
    PrivateKey {
        key_path: String,
        passphrase: Option<String>,
    },
}

impl Default for AuthMethod {
    fn default() -> Self {
        AuthMethod::Password {
            password: String::new(),
        }
    }
}

/// A single SSH connection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConnection {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    #[serde(default)]
    pub group: Option<String>,
}

impl SshConnection {
    /// Create a new connection with a generated ID.
    pub fn new(name: &str, host: &str, port: u16, username: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            host: host.to_string(),
            port,
            username: username.to_string(),
            auth_method: AuthMethod::default(),
            group: None,
        }
    }

    /// A short descriptor for display, e.g. "user@host:port".
    pub fn descriptor(&self) -> String {
        format!("{}@{}:{}", self.username, self.host, self.port)
    }
}

/// The full application configuration, persisted as JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub connections: Vec<SshConnection>,
    #[serde(default)]
    pub groups: Vec<String>,
}

impl AppConfig {
    /// Get the path to the config file.
    /// Uses `~/.config/ssh-mamaged/config.json` on Linux/macOS.
    fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .context("Could not determine config directory")?
            .join("ssh-mamaged");
        Ok(config_dir.join("config.json"))
    }

    /// Load the config from disk, or return default if it doesn't exist.
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            let config = Self::default();
            // Create parent directory and save default config.
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            config.save()?;
            return Ok(config);
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config: AppConfig = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
        Ok(config)
    }

    /// Save the config to disk atomically.
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Atomic write: write to a temp file first, then rename.
        let tmp_path = path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&tmp_path, content)
            .with_context(|| format!("Failed to write temp config file: {}", tmp_path.display()))?;
        fs::rename(&tmp_path, &path)
            .with_context(|| format!("Failed to rename config file to: {}", path.display()))?;
        Ok(())
    }

    /// Add a connection and persist.
    pub fn add_connection(&mut self, conn: SshConnection) -> Result<()> {
        if !self.groups.contains(&conn.name) {
            if let Some(ref group) = conn.group {
                if !self.groups.contains(group) {
                    self.groups.push(group.clone());
                }
            }
        }
        self.connections.push(conn);
        self.save()
    }

    /// Update a connection by ID and persist.
    pub fn update_connection(&mut self, conn: SshConnection) -> Result<()> {
        if let Some(existing) = self.connections.iter_mut().find(|c| c.id == conn.id) {
            *existing = conn;
        }
        self.save()
    }

    /// Remove a connection by ID and persist.
    pub fn remove_connection(&mut self, id: &str) -> Result<()> {
        self.connections.retain(|c| c.id != id);
        self.save()
    }

    /// Get a connection by ID.
    pub fn get_connection(&self, id: &str) -> Option<&SshConnection> {
        self.connections.iter().find(|c| c.id == id)
    }

    /// Get all connection IDs in a given group.
    pub fn connections_in_group(&self, group: &str) -> Vec<&SshConnection> {
        self.connections
            .iter()
            .filter(|c| c.group.as_deref() == Some(group))
            .collect()
    }

    /// Get ungrouped connections.
    pub fn ungrouped_connections(&self) -> Vec<&SshConnection> {
        self.connections
            .iter()
            .filter(|c| c.group.is_none())
            .collect()
    }
}
