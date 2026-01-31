//! WhatsApp Channel Implementation
//!
//! Uses Twilio WhatsApp API for message handling.
//! Alternative: Baileys (WhatsApp Web protocol) for self-hosted.
//!
//! # Configuration
//!
//! Environment variables:
//! - `TWILIO_ACCOUNT_SID`: Twilio account SID
//! - `TWILIO_AUTH_TOKEN`: Twilio auth token
//! - `TWILIO_WHATSAPP_NUMBER`: Your Twilio WhatsApp number (e.g., +14155238886)
//!
//! # Webhook Setup
//!
//! Configure Twilio webhook to POST to: `https://your-domain.com/whatsapp/webhook`

use super::traits::*;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// WhatsApp channel configuration
#[derive(Debug, Clone)]
pub struct WhatsAppConfig {
    /// Twilio Account SID
    pub account_sid: String,
    /// Twilio Auth Token
    pub auth_token: String,
    /// Twilio WhatsApp number (with country code)
    pub whatsapp_number: String,
    /// Webhook URL for incoming messages
    pub webhook_url: Option<String>,
    /// Maximum message length (WhatsApp limit: 4096)
    pub max_message_length: usize,
}

impl WhatsAppConfig {
    /// Load from environment variables
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            account_sid: std::env::var("TWILIO_ACCOUNT_SID")
                .map_err(|_| anyhow::anyhow!("TWILIO_ACCOUNT_SID not set"))?,
            auth_token: std::env::var("TWILIO_AUTH_TOKEN")
                .map_err(|_| anyhow::anyhow!("TWILIO_AUTH_TOKEN not set"))?,
            whatsapp_number: std::env::var("TWILIO_WHATSAPP_NUMBER")
                .map_err(|_| anyhow::anyhow!("TWILIO_WHATSAPP_NUMBER not set"))?,
            webhook_url: std::env::var("WHATSAPP_WEBHOOK_URL").ok(),
            max_message_length: 4096,
        })
    }
}

/// WhatsApp channel implementation
pub struct WhatsAppChannel {
    config: WhatsAppConfig,
    client: reqwest::Client,
    ready: bool,
    /// Active chat sessions (phone numbers)
    active_chats: Arc<RwLock<HashSet<String>>>,
}

impl WhatsAppChannel {
    pub fn new(config: WhatsAppConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            ready: false,
            active_chats: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Create from environment
    pub fn from_env() -> Result<Self> {
        Ok(Self::new(WhatsAppConfig::from_env()?))
    }

    /// Parse incoming Twilio webhook request
    pub fn parse_webhook(&self, form_data: &TwilioWebhookData) -> ChannelMessage {
        ChannelMessage {
            id: form_data.message_sid.clone(),
            channel: "whatsapp".to_string(),
            sender_id: form_data.from.replace("whatsapp:", ""),
            sender_name: form_data.profile_name.clone(),
            chat_id: form_data.from.replace("whatsapp:", ""),
            is_group: false, // Twilio doesn't support groups directly
            content: form_data.body.clone(),
            message_type: if form_data.num_media > 0 {
                MessageType::Image // Simplified - could check media type
            } else {
                MessageType::Text
            },
            media_url: form_data.media_url_0.clone(),
            reply_to: None,
            timestamp: chrono::Utc::now().timestamp(),
            raw: Some(serde_json::to_value(form_data).unwrap_or_default()),
        }
    }

    /// Send message via Twilio API
    async fn send_twilio_message(
        &self,
        to: &str,
        body: &str,
        media_url: Option<&str>,
    ) -> Result<String, ChannelError> {
        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
            self.config.account_sid
        );

        let to_formatted = if to.starts_with("whatsapp:") {
            to.to_string()
        } else {
            format!("whatsapp:{}", to)
        };

        let from_formatted = if self.config.whatsapp_number.starts_with("whatsapp:") {
            self.config.whatsapp_number.clone()
        } else {
            format!("whatsapp:{}", self.config.whatsapp_number)
        };

        let mut form = vec![
            ("From", from_formatted.as_str()),
            ("To", to_formatted.as_str()),
            ("Body", body),
        ];

        let media_url_owned: String;
        if let Some(url) = media_url {
            media_url_owned = url.to_string();
            form.push(("MediaUrl", media_url_owned.as_str()));
        }

        let response = self
            .client
            .post(&url)
            .basic_auth(&self.config.account_sid, Some(&self.config.auth_token))
            .form(&form)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed(e.to_string()))?;

        if response.status().is_success() {
            let result: TwilioMessageResponse = response
                .json()
                .await
                .map_err(|e| ChannelError::Internal(e.to_string()))?;

            // Track active chat
            let mut chats = self.active_chats.write().await;
            chats.insert(to.to_string());

            Ok(result.sid)
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();

            if status.as_u16() == 429 {
                Err(ChannelError::RateLimited(60))
            } else {
                Err(ChannelError::SendFailed(format!(
                    "Twilio error {}: {}",
                    status, error_text
                )))
            }
        }
    }

    /// Split long messages into chunks
    fn split_message(&self, content: &str) -> Vec<String> {
        let max_len = self.config.max_message_length;
        if content.len() <= max_len {
            return vec![content.to_string()];
        }

        let mut chunks = Vec::new();
        let mut current = String::new();

        for line in content.lines() {
            if current.len() + line.len() + 1 > max_len {
                if !current.is_empty() {
                    chunks.push(current);
                    current = String::new();
                }

                // Handle lines longer than max
                if line.len() > max_len {
                    for chunk in line.as_bytes().chunks(max_len) {
                        chunks.push(String::from_utf8_lossy(chunk).to_string());
                    }
                } else {
                    current = line.to_string();
                }
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
}

#[async_trait]
impl Channel for WhatsAppChannel {
    fn name(&self) -> &str {
        "whatsapp"
    }

    fn is_ready(&self) -> bool {
        self.ready
    }

    async fn connect(&mut self) -> Result<(), ChannelError> {
        // Verify Twilio credentials by making a test API call
        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}.json",
            self.config.account_sid
        );

        let response = self
            .client
            .get(&url)
            .basic_auth(&self.config.account_sid, Some(&self.config.auth_token))
            .send()
            .await
            .map_err(|e| ChannelError::ConnectionFailed(e.to_string()))?;

        if response.status().is_success() {
            self.ready = true;
            info!("WhatsApp channel connected via Twilio");
            Ok(())
        } else {
            Err(ChannelError::AuthenticationFailed(
                "Invalid Twilio credentials".to_string(),
            ))
        }
    }

    async fn disconnect(&mut self) -> Result<(), ChannelError> {
        self.ready = false;
        info!("WhatsApp channel disconnected");
        Ok(())
    }

    async fn send(&self, response: ChannelResponse) -> Result<String, ChannelError> {
        if !self.ready {
            return Err(ChannelError::NotReady);
        }

        let chunks = self.split_message(&response.content);
        let mut last_id = String::new();

        for (i, chunk) in chunks.iter().enumerate() {
            // Add media only to first message
            let media_url = if i == 0 {
                response.media.as_ref().and_then(|m| m.url.as_deref())
            } else {
                None
            };

            last_id = self
                .send_twilio_message(&response.chat_id, chunk, media_url)
                .await?;

            // Small delay between chunks to maintain order
            if i < chunks.len() - 1 {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }

        Ok(last_id)
    }

    async fn send_typing(&self, _chat_id: &str) -> Result<(), ChannelError> {
        // WhatsApp/Twilio doesn't support typing indicators
        Ok(())
    }

    async fn edit(&self, _chat_id: &str, _message_id: &str, _content: &str) -> Result<(), ChannelError> {
        // WhatsApp doesn't support message editing
        Err(ChannelError::Internal(
            "WhatsApp doesn't support message editing".to_string(),
        ))
    }

    async fn delete(&self, _chat_id: &str, _message_id: &str) -> Result<(), ChannelError> {
        // WhatsApp doesn't support message deletion via API
        Err(ChannelError::Internal(
            "WhatsApp doesn't support message deletion via API".to_string(),
        ))
    }

    async fn answer_callback(&self, _callback_id: &str, _text: Option<&str>) -> Result<(), ChannelError> {
        // WhatsApp doesn't have callbacks like Telegram
        Ok(())
    }

    async fn broadcast(&self, message: &str) -> Result<(), ChannelError> {
        let chats = self.active_chats.read().await;

        for chat_id in chats.iter() {
            if let Err(e) = self
                .send(ChannelResponse::text(chat_id, message))
                .await
            {
                warn!("Failed to broadcast to {}: {}", chat_id, e);
            }
        }

        Ok(())
    }

    fn config(&self) -> serde_json::Value {
        serde_json::json!({
            "account_sid": &self.config.account_sid[..8], // Partial for security
            "whatsapp_number": &self.config.whatsapp_number,
            "webhook_url": &self.config.webhook_url,
        })
    }
}

#[async_trait]
impl ChannelSender for WhatsAppChannel {
    async fn send_text(&self, chat_id: &str, text: &str) -> Result<String, ChannelError> {
        self.send(ChannelResponse::text(chat_id, text)).await
    }

    async fn send_image(&self, chat_id: &str, url: &str, caption: Option<&str>) -> Result<String, ChannelError> {
        let response = ChannelResponse {
            chat_id: chat_id.to_string(),
            content: caption.unwrap_or("").to_string(),
            reply_to: None,
            buttons: Vec::new(),
            media: Some(MediaAttachment {
                media_type: MessageType::Image,
                url: Some(url.to_string()),
                data: None,
                filename: None,
                caption: caption.map(|s| s.to_string()),
            }),
            parse_mode: ParseMode::Plain,
        };
        self.send(response).await
    }

    async fn send_document(&self, chat_id: &str, url: &str, filename: &str) -> Result<String, ChannelError> {
        let response = ChannelResponse {
            chat_id: chat_id.to_string(),
            content: filename.to_string(),
            reply_to: None,
            buttons: Vec::new(),
            media: Some(MediaAttachment {
                media_type: MessageType::Document,
                url: Some(url.to_string()),
                data: None,
                filename: Some(filename.to_string()),
                caption: None,
            }),
            parse_mode: ParseMode::Plain,
        };
        self.send(response).await
    }
}

/// Twilio webhook incoming data
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TwilioWebhookData {
    pub message_sid: String,
    pub account_sid: String,
    pub from: String,
    pub to: String,
    pub body: String,
    #[serde(default)]
    pub num_media: u32,
    pub media_url_0: Option<String>,
    pub profile_name: Option<String>,
}

/// Twilio API response
#[derive(Debug, Deserialize)]
struct TwilioMessageResponse {
    sid: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_splitting() {
        let config = WhatsAppConfig {
            account_sid: "test".to_string(),
            auth_token: "test".to_string(),
            whatsapp_number: "+1234567890".to_string(),
            webhook_url: None,
            max_message_length: 50,
        };
        let channel = WhatsAppChannel::new(config);

        let short = "Hello";
        assert_eq!(channel.split_message(short).len(), 1);

        let long = "A".repeat(100);
        let chunks = channel.split_message(&long);
        assert!(chunks.len() >= 2);
    }

    #[test]
    fn test_phone_formatting() {
        let msg = ChannelMessage::text("whatsapp", "+1234567890", "+1234567890", "test");
        assert!(msg.sender_id_numeric() != 0);
    }
}
