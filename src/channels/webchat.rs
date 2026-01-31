//! WebChat Channel Implementation
//!
//! WebSocket-based chat interface for web browsers.
//!
//! # Configuration
//!
//! Environment variables:
//! - `WEBCHAT_PORT`: WebSocket server port (default: 8765)
//! - `WEBCHAT_ALLOWED_ORIGINS`: Comma-separated allowed origins for CORS

use super::traits::*;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

/// WebChat configuration
#[derive(Debug, Clone)]
pub struct WebChatConfig {
    /// WebSocket port
    pub port: u16,
    /// Allowed CORS origins
    pub allowed_origins: Vec<String>,
    /// Maximum message length
    pub max_message_length: usize,
    /// Session timeout in seconds
    pub session_timeout_secs: u64,
}

impl Default for WebChatConfig {
    fn default() -> Self {
        Self {
            port: 8765,
            allowed_origins: vec!["*".to_string()],
            max_message_length: 10000,
            session_timeout_secs: 3600,
        }
    }
}

impl WebChatConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            port: std::env::var("WEBCHAT_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(8765),
            allowed_origins: std::env::var("WEBCHAT_ALLOWED_ORIGINS")
                .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_else(|_| vec!["*".to_string()]),
            max_message_length: 10000,
            session_timeout_secs: 3600,
        })
    }
}

/// WebSocket session
#[derive(Debug)]
struct WebSession {
    id: String,
    user_id: String,
    sender: mpsc::Sender<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    last_activity: chrono::DateTime<chrono::Utc>,
}

/// WebChat channel implementation
pub struct WebChatChannel {
    config: WebChatConfig,
    ready: bool,
    /// Active sessions: session_id -> WebSession
    sessions: Arc<RwLock<HashMap<String, WebSession>>>,
    /// Message queue for outgoing messages: session_id -> messages
    outbound_queue: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

impl WebChatChannel {
    pub fn new(config: WebChatConfig) -> Self {
        Self {
            config,
            ready: false,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            outbound_queue: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn from_env() -> Result<Self> {
        Ok(Self::new(WebChatConfig::from_env()?))
    }

    /// Register a new WebSocket session
    pub async fn register_session(
        &self,
        session_id: &str,
        user_id: &str,
        sender: mpsc::Sender<String>,
    ) {
        let mut sessions = self.sessions.write().await;
        let now = chrono::Utc::now();

        sessions.insert(
            session_id.to_string(),
            WebSession {
                id: session_id.to_string(),
                user_id: user_id.to_string(),
                sender,
                created_at: now,
                last_activity: now,
            },
        );

        info!("WebChat session registered: {}", session_id);
    }

    /// Unregister a session
    pub async fn unregister_session(&self, session_id: &str) {
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id);
        info!("WebChat session unregistered: {}", session_id);
    }

    /// Update session activity
    pub async fn touch_session(&self, session_id: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session.last_activity = chrono::Utc::now();
        }
    }

    /// Parse incoming WebSocket message
    pub fn parse_message(&self, session_id: &str, user_id: &str, data: &WebChatIncoming) -> ChannelMessage {
        ChannelMessage {
            id: data.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            channel: "webchat".to_string(),
            sender_id: user_id.to_string(),
            sender_name: data.username.clone(),
            chat_id: session_id.to_string(),
            is_group: false,
            content: data.content.clone(),
            message_type: MessageType::Text,
            media_url: None,
            reply_to: data.reply_to.clone(),
            timestamp: chrono::Utc::now().timestamp(),
            raw: Some(serde_json::to_value(data).unwrap_or_default()),
        }
    }

    /// Format outgoing message
    fn format_outgoing(&self, response: &ChannelResponse) -> String {
        let msg = WebChatOutgoing {
            id: uuid::Uuid::new_v4().to_string(),
            content: response.content.clone(),
            message_type: "text".to_string(),
            timestamp: chrono::Utc::now().timestamp(),
            buttons: if response.buttons.is_empty() {
                None
            } else {
                Some(
                    response
                        .buttons
                        .iter()
                        .flatten()
                        .map(|b| WebChatButton {
                            text: b.text.clone(),
                            action: b.callback_data.clone().unwrap_or_default(),
                            url: b.url.clone(),
                        })
                        .collect(),
                )
            },
        };
        serde_json::to_string(&msg).unwrap_or_default()
    }

    /// Clean up expired sessions
    pub async fn cleanup_expired(&self) {
        let timeout = chrono::Duration::seconds(self.config.session_timeout_secs as i64);
        let now = chrono::Utc::now();

        let mut sessions = self.sessions.write().await;
        let expired: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| now - s.last_activity > timeout)
            .map(|(id, _)| id.clone())
            .collect();

        for id in expired {
            sessions.remove(&id);
            info!("WebChat session expired: {}", id);
        }
    }
}

#[async_trait]
impl Channel for WebChatChannel {
    fn name(&self) -> &str {
        "webchat"
    }

    fn is_ready(&self) -> bool {
        self.ready
    }

    async fn connect(&mut self) -> Result<(), ChannelError> {
        // WebSocket server would be started separately
        // This just marks the channel as ready
        self.ready = true;
        info!("WebChat channel ready on port {}", self.config.port);
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), ChannelError> {
        self.ready = false;
        // Close all sessions
        let mut sessions = self.sessions.write().await;
        sessions.clear();
        info!("WebChat channel disconnected");
        Ok(())
    }

    async fn send(&self, response: ChannelResponse) -> Result<String, ChannelError> {
        if !self.ready {
            return Err(ChannelError::NotReady);
        }

        let sessions = self.sessions.read().await;
        let session = sessions
            .get(&response.chat_id)
            .ok_or_else(|| ChannelError::InvalidRecipient(response.chat_id.clone()))?;

        let message = self.format_outgoing(&response);
        let message_id = uuid::Uuid::new_v4().to_string();

        session
            .sender
            .send(message)
            .await
            .map_err(|e| ChannelError::SendFailed(e.to_string()))?;

        Ok(message_id)
    }

    async fn send_typing(&self, chat_id: &str) -> Result<(), ChannelError> {
        let sessions = self.sessions.read().await;
        if let Some(session) = sessions.get(chat_id) {
            let typing_msg = serde_json::json!({
                "type": "typing",
                "timestamp": chrono::Utc::now().timestamp(),
            });
            let _ = session.sender.send(typing_msg.to_string()).await;
        }
        Ok(())
    }

    async fn edit(&self, chat_id: &str, message_id: &str, content: &str) -> Result<(), ChannelError> {
        let sessions = self.sessions.read().await;
        if let Some(session) = sessions.get(chat_id) {
            let edit_msg = serde_json::json!({
                "type": "edit",
                "message_id": message_id,
                "content": content,
                "timestamp": chrono::Utc::now().timestamp(),
            });
            session
                .sender
                .send(edit_msg.to_string())
                .await
                .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
        }
        Ok(())
    }

    async fn delete(&self, chat_id: &str, message_id: &str) -> Result<(), ChannelError> {
        let sessions = self.sessions.read().await;
        if let Some(session) = sessions.get(chat_id) {
            let delete_msg = serde_json::json!({
                "type": "delete",
                "message_id": message_id,
                "timestamp": chrono::Utc::now().timestamp(),
            });
            session
                .sender
                .send(delete_msg.to_string())
                .await
                .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
        }
        Ok(())
    }

    async fn answer_callback(&self, callback_id: &str, text: Option<&str>) -> Result<(), ChannelError> {
        // WebChat handles callbacks via the same WebSocket
        // callback_id format: "session_id:action_id"
        if let Some((session_id, _action_id)) = callback_id.split_once(':') {
            let sessions = self.sessions.read().await;
            if let Some(session) = sessions.get(session_id) {
                let ack_msg = serde_json::json!({
                    "type": "callback_ack",
                    "callback_id": callback_id,
                    "text": text,
                    "timestamp": chrono::Utc::now().timestamp(),
                });
                let _ = session.sender.send(ack_msg.to_string()).await;
            }
        }
        Ok(())
    }

    async fn broadcast(&self, message: &str) -> Result<(), ChannelError> {
        let sessions = self.sessions.read().await;

        for session in sessions.values() {
            let broadcast_msg = serde_json::json!({
                "type": "broadcast",
                "content": message,
                "timestamp": chrono::Utc::now().timestamp(),
            });
            let _ = session.sender.send(broadcast_msg.to_string()).await;
        }

        Ok(())
    }

    fn config(&self) -> serde_json::Value {
        serde_json::json!({
            "port": self.config.port,
            "allowed_origins": &self.config.allowed_origins,
            "active_sessions": 0, // Would need async to get real count
        })
    }
}

#[async_trait]
impl ChannelSender for WebChatChannel {
    async fn send_text(&self, chat_id: &str, text: &str) -> Result<String, ChannelError> {
        self.send(ChannelResponse::text(chat_id, text)).await
    }

    async fn send_image(&self, chat_id: &str, url: &str, caption: Option<&str>) -> Result<String, ChannelError> {
        let sessions = self.sessions.read().await;
        if let Some(session) = sessions.get(chat_id) {
            let img_msg = serde_json::json!({
                "type": "image",
                "url": url,
                "caption": caption,
                "timestamp": chrono::Utc::now().timestamp(),
            });
            session
                .sender
                .send(img_msg.to_string())
                .await
                .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
            Ok(uuid::Uuid::new_v4().to_string())
        } else {
            Err(ChannelError::InvalidRecipient(chat_id.to_string()))
        }
    }

    async fn send_document(&self, chat_id: &str, url: &str, filename: &str) -> Result<String, ChannelError> {
        let sessions = self.sessions.read().await;
        if let Some(session) = sessions.get(chat_id) {
            let doc_msg = serde_json::json!({
                "type": "document",
                "url": url,
                "filename": filename,
                "timestamp": chrono::Utc::now().timestamp(),
            });
            session
                .sender
                .send(doc_msg.to_string())
                .await
                .map_err(|e| ChannelError::SendFailed(e.to_string()))?;
            Ok(uuid::Uuid::new_v4().to_string())
        } else {
            Err(ChannelError::InvalidRecipient(chat_id.to_string()))
        }
    }
}

/// Incoming WebSocket message format
#[derive(Debug, Deserialize, Serialize)]
pub struct WebChatIncoming {
    pub id: Option<String>,
    pub content: String,
    pub username: Option<String>,
    pub reply_to: Option<String>,
}

/// Outgoing WebSocket message format
#[derive(Debug, Serialize)]
struct WebChatOutgoing {
    id: String,
    content: String,
    message_type: String,
    timestamp: i64,
    buttons: Option<Vec<WebChatButton>>,
}

#[derive(Debug, Serialize)]
struct WebChatButton {
    text: String,
    action: String,
    url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = WebChatConfig::default();
        assert_eq!(config.port, 8765);
    }

    #[test]
    fn test_parse_message() {
        let channel = WebChatChannel::new(WebChatConfig::default());
        let incoming = WebChatIncoming {
            id: Some("msg1".to_string()),
            content: "Hello".to_string(),
            username: Some("user1".to_string()),
            reply_to: None,
        };

        let msg = channel.parse_message("session1", "user1", &incoming);
        assert_eq!(msg.content, "Hello");
        assert_eq!(msg.channel, "webchat");
    }
}
