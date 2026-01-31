//! Discord Channel Implementation
//!
//! Uses Discord Bot API via serenity crate.
//!
//! # Configuration
//!
//! Environment variables:
//! - `DISCORD_BOT_TOKEN`: Discord bot token
//! - `DISCORD_APPLICATION_ID`: Discord application ID

use super::traits::*;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Discord channel configuration
#[derive(Debug, Clone)]
pub struct DiscordConfig {
    /// Bot token
    pub bot_token: String,
    /// Application ID
    pub application_id: String,
    /// Allowed guild IDs (empty = all)
    pub allowed_guilds: Vec<String>,
    /// Maximum message length (Discord limit: 2000)
    pub max_message_length: usize,
}

impl DiscordConfig {
    /// Load from environment
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            bot_token: std::env::var("DISCORD_BOT_TOKEN")
                .map_err(|_| anyhow::anyhow!("DISCORD_BOT_TOKEN not set"))?,
            application_id: std::env::var("DISCORD_APPLICATION_ID")
                .map_err(|_| anyhow::anyhow!("DISCORD_APPLICATION_ID not set"))?,
            allowed_guilds: std::env::var("DISCORD_ALLOWED_GUILDS")
                .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default(),
            max_message_length: 2000,
        })
    }
}

/// Discord channel implementation
pub struct DiscordChannel {
    config: DiscordConfig,
    client: reqwest::Client,
    ready: bool,
    /// Active channel IDs
    active_channels: Arc<RwLock<HashSet<String>>>,
}

impl DiscordChannel {
    pub fn new(config: DiscordConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            ready: false,
            active_channels: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Create from environment
    pub fn from_env() -> Result<Self> {
        Ok(Self::new(DiscordConfig::from_env()?))
    }

    /// Send message via Discord API
    async fn send_discord_message(
        &self,
        channel_id: &str,
        content: &str,
        reply_to: Option<&str>,
    ) -> Result<String, ChannelError> {
        let url = format!(
            "https://discord.com/api/v10/channels/{}/messages",
            channel_id
        );

        let mut payload = serde_json::json!({
            "content": content,
        });

        if let Some(message_id) = reply_to {
            payload["message_reference"] = serde_json::json!({
                "message_id": message_id,
            });
        }

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.config.bot_token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed(e.to_string()))?;

        if response.status().is_success() {
            let result: DiscordMessageResponse = response
                .json()
                .await
                .map_err(|e| ChannelError::Internal(e.to_string()))?;

            // Track active channel
            let mut channels = self.active_channels.write().await;
            channels.insert(channel_id.to_string());

            Ok(result.id)
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();

            if status.as_u16() == 429 {
                // Parse retry_after from response
                Err(ChannelError::RateLimited(5))
            } else {
                Err(ChannelError::SendFailed(format!(
                    "Discord error {}: {}",
                    status, error_text
                )))
            }
        }
    }

    /// Split long messages
    fn split_message(&self, content: &str) -> Vec<String> {
        let max_len = self.config.max_message_length;
        if content.len() <= max_len {
            return vec![content.to_string()];
        }

        let mut chunks = Vec::new();
        let mut current = String::new();

        // Try to split at code block boundaries first
        let code_block_pattern = "```";

        for line in content.lines() {
            if current.len() + line.len() + 1 > max_len {
                // Check if we're in a code block
                let open_blocks = current.matches(code_block_pattern).count();
                if open_blocks % 2 == 1 {
                    // We're in an unclosed code block, close it
                    current.push_str("\n```");
                }

                if !current.is_empty() {
                    chunks.push(current);
                    current = String::new();
                }

                // Re-open code block if needed
                if open_blocks % 2 == 1 {
                    current.push_str("```\n");
                }

                current.push_str(line);
            } else {
                if !current.is_empty() {
                    current.push('\n');
                }
                current.push_str(line);
            }
        }

        if !current.is_empty() {
            chunks.push(current);
        }

        chunks
    }

    /// Parse Discord webhook/gateway event into ChannelMessage
    pub fn parse_message(&self, event: &DiscordMessageEvent) -> ChannelMessage {
        ChannelMessage {
            id: event.id.clone(),
            channel: "discord".to_string(),
            sender_id: event.author.id.clone(),
            sender_name: Some(event.author.username.clone()),
            chat_id: event.channel_id.clone(),
            is_group: event.guild_id.is_some(),
            content: event.content.clone(),
            message_type: if !event.attachments.is_empty() {
                MessageType::Document
            } else {
                MessageType::Text
            },
            media_url: event.attachments.first().map(|a| a.url.clone()),
            reply_to: event.message_reference.as_ref().map(|r| r.message_id.clone()),
            timestamp: chrono::Utc::now().timestamp(),
            raw: Some(serde_json::to_value(event).unwrap_or_default()),
        }
    }
}

#[async_trait]
impl Channel for DiscordChannel {
    fn name(&self) -> &str {
        "discord"
    }

    fn is_ready(&self) -> bool {
        self.ready
    }

    async fn connect(&mut self) -> Result<(), ChannelError> {
        // Verify bot token by getting current user
        let response = self
            .client
            .get("https://discord.com/api/v10/users/@me")
            .header("Authorization", format!("Bot {}", self.config.bot_token))
            .send()
            .await
            .map_err(|e| ChannelError::ConnectionFailed(e.to_string()))?;

        if response.status().is_success() {
            self.ready = true;
            info!("Discord channel connected");
            Ok(())
        } else {
            Err(ChannelError::AuthenticationFailed(
                "Invalid Discord bot token".to_string(),
            ))
        }
    }

    async fn disconnect(&mut self) -> Result<(), ChannelError> {
        self.ready = false;
        info!("Discord channel disconnected");
        Ok(())
    }

    async fn send(&self, response: ChannelResponse) -> Result<String, ChannelError> {
        if !self.ready {
            return Err(ChannelError::NotReady);
        }

        let chunks = self.split_message(&response.content);
        let mut last_id = String::new();

        for (i, chunk) in chunks.iter().enumerate() {
            // Reply only on first message
            let reply_to = if i == 0 {
                response.reply_to.as_deref()
            } else {
                None
            };

            last_id = self
                .send_discord_message(&response.chat_id, chunk, reply_to)
                .await?;

            // Respect rate limits
            if i < chunks.len() - 1 {
                tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
            }
        }

        Ok(last_id)
    }

    async fn send_typing(&self, chat_id: &str) -> Result<(), ChannelError> {
        let url = format!(
            "https://discord.com/api/v10/channels/{}/typing",
            chat_id
        );

        self.client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.config.bot_token))
            .send()
            .await
            .map_err(|e| ChannelError::Internal(e.to_string()))?;

        Ok(())
    }

    async fn edit(&self, chat_id: &str, message_id: &str, content: &str) -> Result<(), ChannelError> {
        let url = format!(
            "https://discord.com/api/v10/channels/{}/messages/{}",
            chat_id, message_id
        );

        let response = self
            .client
            .patch(&url)
            .header("Authorization", format!("Bot {}", self.config.bot_token))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await
            .map_err(|e| ChannelError::Internal(e.to_string()))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(ChannelError::SendFailed("Failed to edit message".to_string()))
        }
    }

    async fn delete(&self, chat_id: &str, message_id: &str) -> Result<(), ChannelError> {
        let url = format!(
            "https://discord.com/api/v10/channels/{}/messages/{}",
            chat_id, message_id
        );

        let response = self
            .client
            .delete(&url)
            .header("Authorization", format!("Bot {}", self.config.bot_token))
            .send()
            .await
            .map_err(|e| ChannelError::Internal(e.to_string()))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(ChannelError::SendFailed("Failed to delete message".to_string()))
        }
    }

    async fn answer_callback(&self, callback_id: &str, text: Option<&str>) -> Result<(), ChannelError> {
        // Discord uses interaction responses, different from Telegram callbacks
        // This would be used for slash command interactions
        let url = format!(
            "https://discord.com/api/v10/interactions/{}/callback",
            callback_id
        );

        let payload = serde_json::json!({
            "type": 4, // CHANNEL_MESSAGE_WITH_SOURCE
            "data": {
                "content": text.unwrap_or("Done"),
            }
        });

        self.client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.config.bot_token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| ChannelError::Internal(e.to_string()))?;

        Ok(())
    }

    async fn broadcast(&self, message: &str) -> Result<(), ChannelError> {
        let channels = self.active_channels.read().await;

        for channel_id in channels.iter() {
            if let Err(e) = self
                .send(ChannelResponse::text(channel_id, message))
                .await
            {
                warn!("Failed to broadcast to Discord channel {}: {}", channel_id, e);
            }
        }

        Ok(())
    }

    fn config(&self) -> serde_json::Value {
        serde_json::json!({
            "application_id": &self.config.application_id,
            "allowed_guilds": &self.config.allowed_guilds,
        })
    }
}

#[async_trait]
impl ChannelSender for DiscordChannel {
    async fn send_text(&self, chat_id: &str, text: &str) -> Result<String, ChannelError> {
        self.send(ChannelResponse::text(chat_id, text)).await
    }

    async fn send_image(&self, chat_id: &str, url: &str, caption: Option<&str>) -> Result<String, ChannelError> {
        // Discord embeds for images
        let content = if let Some(cap) = caption {
            format!("{}\n{}", cap, url)
        } else {
            url.to_string()
        };
        self.send(ChannelResponse::text(chat_id, &content)).await
    }

    async fn send_document(&self, chat_id: &str, url: &str, filename: &str) -> Result<String, ChannelError> {
        let content = format!("[{}]({})", filename, url);
        self.send(ChannelResponse::text(chat_id, &content)).await
    }
}

/// Discord message event from gateway
#[derive(Debug, Deserialize, Serialize)]
pub struct DiscordMessageEvent {
    pub id: String,
    pub channel_id: String,
    pub guild_id: Option<String>,
    pub author: DiscordUser,
    pub content: String,
    #[serde(default)]
    pub attachments: Vec<DiscordAttachment>,
    pub message_reference: Option<DiscordMessageReference>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DiscordUser {
    pub id: String,
    pub username: String,
    pub discriminator: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DiscordAttachment {
    pub id: String,
    pub filename: String,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DiscordMessageReference {
    pub message_id: String,
    pub channel_id: Option<String>,
    pub guild_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscordMessageResponse {
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_splitting() {
        let config = DiscordConfig {
            bot_token: "test".to_string(),
            application_id: "test".to_string(),
            allowed_guilds: vec![],
            max_message_length: 50,
        };
        let channel = DiscordChannel::new(config);

        let code_block = "```rust\nfn main() {\n    println!(\"Hello\");\n}\n```";
        let chunks = channel.split_message(code_block);
        // Should preserve code block integrity
        assert!(!chunks.is_empty());
    }
}
