//! Credential Vault
//!
//! Secure storage for API keys, tokens, and SSH credentials.
//! Uses AES-256-GCM for encryption with PBKDF2 key derivation.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;
use zeroize::Zeroize;

/// Vault errors
#[derive(Error, Debug)]
pub enum VaultError {
    #[error("Encryption failed: {0}")]
    Encryption(String),
    #[error("Decryption failed: {0}")]
    Decryption(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Credential not found: {0}")]
    NotFound(String),
    #[error("Vault locked")]
    Locked,
    #[error("Invalid master key")]
    InvalidKey,
}

/// Type of credential stored
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CredentialType {
    ApiKey,
    Token,
    SshKey,
    Password,
    Certificate,
    Custom(String),
}

/// A stored credential with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub name: String,
    pub credential_type: CredentialType,
    #[serde(skip_serializing)]
    pub value: String,
    pub encrypted_value: String,
    pub nonce: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub expires_at: Option<i64>,
    pub metadata: HashMap<String, String>,
}

/// Encrypted vault file format
#[derive(Debug, Serialize, Deserialize)]
struct VaultFile {
    version: u32,
    salt: String,
    credentials: Vec<Credential>,
}

/// Secure credential vault with AES-256-GCM encryption
pub struct CredentialVault {
    path: PathBuf,
    cipher: Option<Aes256Gcm>,
    credentials: HashMap<String, Credential>,
    salt: [u8; 32],
    locked: bool,
}

impl CredentialVault {
    /// Create or open a vault at the given path
    pub fn new(path: PathBuf) -> Result<Self, VaultError> {
        let mut salt = [0u8; 32];

        // Load existing vault or create new
        if path.exists() {
            let data = std::fs::read_to_string(&path)?;
            let vault_file: VaultFile = serde_json::from_str(&data)?;

            let salt_bytes = BASE64.decode(&vault_file.salt)
                .map_err(|e| VaultError::Encryption(e.to_string()))?;
            salt.copy_from_slice(&salt_bytes);

            let mut credentials = HashMap::new();
            for cred in vault_file.credentials {
                credentials.insert(cred.name.clone(), cred);
            }

            Ok(Self {
                path,
                cipher: None,
                credentials,
                salt,
                locked: true,
            })
        } else {
            // Generate new salt
            OsRng.fill_bytes(&mut salt);

            Ok(Self {
                path,
                cipher: None,
                credentials: HashMap::new(),
                salt,
                locked: true,
            })
        }
    }

    /// Open vault with default path (~/.claudebot/vault.json)
    pub fn open_default() -> Result<Self, VaultError> {
        let path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".claudebot")
            .join("vault.json");

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        Self::new(path)
    }

    /// Unlock the vault with a master password
    pub fn unlock(&mut self, master_password: &str) -> Result<(), VaultError> {
        let key = self.derive_key(master_password);
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| VaultError::Encryption(e.to_string()))?;

        // Try to decrypt existing credentials to verify key
        if !self.credentials.is_empty() {
            let test_cred = self.credentials.values().next().unwrap();
            let nonce_bytes = BASE64.decode(&test_cred.nonce)
                .map_err(|e| VaultError::Decryption(e.to_string()))?;
            let ciphertext = BASE64.decode(&test_cred.encrypted_value)
                .map_err(|e| VaultError::Decryption(e.to_string()))?;

            let nonce = Nonce::from_slice(&nonce_bytes);
            cipher.decrypt(nonce, ciphertext.as_ref())
                .map_err(|_| VaultError::InvalidKey)?;
        }

        self.cipher = Some(cipher);
        self.locked = false;

        // Decrypt all credentials
        self.decrypt_all()?;

        Ok(())
    }

    /// Lock the vault, clearing decrypted values from memory
    pub fn lock(&mut self) {
        // Clear decrypted values
        for cred in self.credentials.values_mut() {
            cred.value.zeroize();
            cred.value = String::new();
        }
        self.cipher = None;
        self.locked = true;
    }

    /// Check if vault is locked
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Store a credential
    pub fn store(
        &mut self,
        name: &str,
        value: &str,
        credential_type: CredentialType,
        metadata: Option<HashMap<String, String>>,
        expires_at: Option<i64>,
    ) -> Result<(), VaultError> {
        if self.locked {
            return Err(VaultError::Locked);
        }

        let cipher = self.cipher.as_ref().ok_or(VaultError::Locked)?;

        // Generate random nonce
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Encrypt value
        let ciphertext = cipher.encrypt(nonce, value.as_bytes())
            .map_err(|e| VaultError::Encryption(e.to_string()))?;

        let now = chrono::Utc::now().timestamp();

        let credential = Credential {
            name: name.to_string(),
            credential_type,
            value: value.to_string(),
            encrypted_value: BASE64.encode(&ciphertext),
            nonce: BASE64.encode(nonce_bytes),
            created_at: now,
            updated_at: now,
            expires_at,
            metadata: metadata.unwrap_or_default(),
        };

        self.credentials.insert(name.to_string(), credential);
        self.save()?;

        Ok(())
    }

    /// Get a credential by name
    pub fn get(&self, name: &str) -> Result<&Credential, VaultError> {
        if self.locked {
            return Err(VaultError::Locked);
        }

        self.credentials.get(name)
            .ok_or_else(|| VaultError::NotFound(name.to_string()))
    }

    /// Get credential value (decrypted)
    pub fn get_value(&self, name: &str) -> Result<&str, VaultError> {
        let cred = self.get(name)?;
        Ok(&cred.value)
    }

    /// Delete a credential
    pub fn delete(&mut self, name: &str) -> Result<(), VaultError> {
        if self.locked {
            return Err(VaultError::Locked);
        }

        if let Some(mut cred) = self.credentials.remove(name) {
            cred.value.zeroize();
        }

        self.save()?;
        Ok(())
    }

    /// List all credential names
    pub fn list(&self) -> Vec<&str> {
        self.credentials.keys().map(|s| s.as_str()).collect()
    }

    /// List credentials by type
    pub fn list_by_type(&self, credential_type: &CredentialType) -> Vec<&Credential> {
        self.credentials.values()
            .filter(|c| &c.credential_type == credential_type)
            .collect()
    }

    /// Check if a credential exists
    pub fn exists(&self, name: &str) -> bool {
        self.credentials.contains_key(name)
    }

    /// Check if a credential is expired
    pub fn is_expired(&self, name: &str) -> bool {
        if let Some(cred) = self.credentials.get(name) {
            if let Some(expires_at) = cred.expires_at {
                return chrono::Utc::now().timestamp() > expires_at;
            }
        }
        false
    }

    /// Get credentials expiring within the given duration
    pub fn expiring_soon(&self, within_secs: i64) -> Vec<&Credential> {
        let deadline = chrono::Utc::now().timestamp() + within_secs;
        self.credentials.values()
            .filter(|c| {
                if let Some(expires_at) = c.expires_at {
                    expires_at <= deadline
                } else {
                    false
                }
            })
            .collect()
    }

    /// Derive encryption key from password using PBKDF2-like approach
    fn derive_key(&self, password: &str) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        hasher.update(&self.salt);

        // Multiple rounds for key stretching
        let mut result = hasher.finalize();
        for _ in 0..10000 {
            let mut hasher = Sha256::new();
            hasher.update(&result);
            hasher.update(&self.salt);
            result = hasher.finalize();
        }

        let mut key = [0u8; 32];
        key.copy_from_slice(&result);
        key
    }

    /// Decrypt all stored credentials
    fn decrypt_all(&mut self) -> Result<(), VaultError> {
        let cipher = self.cipher.as_ref().ok_or(VaultError::Locked)?;

        for cred in self.credentials.values_mut() {
            let nonce_bytes = BASE64.decode(&cred.nonce)
                .map_err(|e| VaultError::Decryption(e.to_string()))?;
            let ciphertext = BASE64.decode(&cred.encrypted_value)
                .map_err(|e| VaultError::Decryption(e.to_string()))?;

            let nonce = Nonce::from_slice(&nonce_bytes);
            let plaintext = cipher.decrypt(nonce, ciphertext.as_ref())
                .map_err(|e| VaultError::Decryption(e.to_string()))?;

            cred.value = String::from_utf8(plaintext)
                .map_err(|e| VaultError::Decryption(e.to_string()))?;
        }

        Ok(())
    }

    /// Save vault to disk
    fn save(&self) -> Result<(), VaultError> {
        let vault_file = VaultFile {
            version: 1,
            salt: BASE64.encode(self.salt),
            credentials: self.credentials.values().cloned().collect(),
        };

        let data = serde_json::to_string_pretty(&vault_file)?;

        // Write atomically via temp file
        let temp_path = self.path.with_extension("tmp");
        std::fs::write(&temp_path, &data)?;
        std::fs::rename(&temp_path, &self.path)?;

        Ok(())
    }

    /// Export credentials (for backup)
    pub fn export(&self, include_values: bool) -> Result<String, VaultError> {
        if self.locked && include_values {
            return Err(VaultError::Locked);
        }

        #[derive(Serialize)]
        struct ExportEntry {
            name: String,
            credential_type: CredentialType,
            value: Option<String>,
            created_at: i64,
            expires_at: Option<i64>,
            metadata: HashMap<String, String>,
        }

        let entries: Vec<ExportEntry> = self.credentials.values()
            .map(|c| ExportEntry {
                name: c.name.clone(),
                credential_type: c.credential_type.clone(),
                value: if include_values { Some(c.value.clone()) } else { None },
                created_at: c.created_at,
                expires_at: c.expires_at,
                metadata: c.metadata.clone(),
            })
            .collect();

        Ok(serde_json::to_string_pretty(&entries)?)
    }
}

impl Drop for CredentialVault {
    fn drop(&mut self) {
        // Ensure sensitive data is cleared
        self.lock();
    }
}

/// Convenience functions for common credential types
impl CredentialVault {
    /// Store an API key
    pub fn store_api_key(&mut self, name: &str, key: &str) -> Result<(), VaultError> {
        self.store(name, key, CredentialType::ApiKey, None, None)
    }

    /// Store a token with optional expiration
    pub fn store_token(&mut self, name: &str, token: &str, expires_at: Option<i64>) -> Result<(), VaultError> {
        self.store(name, token, CredentialType::Token, None, expires_at)
    }

    /// Store an SSH private key
    pub fn store_ssh_key(&mut self, name: &str, key: &str, passphrase: Option<&str>) -> Result<(), VaultError> {
        let mut metadata = HashMap::new();
        if let Some(pass) = passphrase {
            metadata.insert("has_passphrase".to_string(), "true".to_string());
            // Store passphrase encrypted separately
            self.store(&format!("{}_passphrase", name), pass, CredentialType::Password, None, None)?;
        }
        self.store(name, key, CredentialType::SshKey, Some(metadata), None)
    }

    /// Get GitHub token if stored
    pub fn get_github_token(&self) -> Option<&str> {
        self.get_value("github_token").ok()
    }

    /// Get Anthropic API key if stored
    pub fn get_anthropic_key(&self) -> Option<&str> {
        self.get_value("anthropic_api_key").ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_vault_basic_operations() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_vault.json");

        let mut vault = CredentialVault::new(path.clone()).unwrap();
        vault.unlock("test_password").unwrap();

        // Store credential
        vault.store_api_key("test_key", "secret123").unwrap();

        // Retrieve credential
        let value = vault.get_value("test_key").unwrap();
        assert_eq!(value, "secret123");

        // Lock and unlock
        vault.lock();
        assert!(vault.is_locked());
        assert!(vault.get_value("test_key").is_err());

        vault.unlock("test_password").unwrap();
        let value = vault.get_value("test_key").unwrap();
        assert_eq!(value, "secret123");
    }

    #[test]
    fn test_vault_persistence() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("persist_vault.json");

        // Create and store
        {
            let mut vault = CredentialVault::new(path.clone()).unwrap();
            vault.unlock("password123").unwrap();
            vault.store_api_key("persistent_key", "persistent_value").unwrap();
        }

        // Reopen and verify
        {
            let mut vault = CredentialVault::new(path).unwrap();
            vault.unlock("password123").unwrap();
            let value = vault.get_value("persistent_key").unwrap();
            assert_eq!(value, "persistent_value");
        }
    }

    #[test]
    fn test_invalid_password() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("invalid_vault.json");

        // Create with one password
        {
            let mut vault = CredentialVault::new(path.clone()).unwrap();
            vault.unlock("correct_password").unwrap();
            vault.store_api_key("key", "value").unwrap();
        }

        // Try wrong password
        {
            let mut vault = CredentialVault::new(path).unwrap();
            let result = vault.unlock("wrong_password");
            assert!(matches!(result, Err(VaultError::InvalidKey)));
        }
    }
}
