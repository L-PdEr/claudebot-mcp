//! Channel Trait Definitions
//!
//! Universal interfaces for all messaging channels.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Error types for channel operations
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("Send failed: {0}")]
    SendFailed(String),

    #[error("Rate limited: retry after {0} seconds")]
    RateLimited(u64),

    #[error("Invalid recipient: {0}")]
    InvalidRecipient(String),

    #[error("Media upload failed: {0}")]
    MediaUploadFailed(String),

    #[error("Channel not ready")]
    NotReady,

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Message type enumeration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageType {
    Text,
    Image,
    Audio,
    Video,
    Document,
    Location,
    Contact,
    Sticker,
    Voice,
    Unknown,
}

/// Universal message representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    /// Unique message ID (channel-specific format)
    pub id: String,

    /// Channel name (telegram, whatsapp, discord, etc.)
    pub channel: String,

    /// Sender identifier (normalized)
    pub sender_id: String,

    /// Sender display name
    pub sender_name: Option<String>,

    /// Chat/conversation ID
    pub chat_id: String,

    /// Is this a group chat?
    pub is_group: bool,

    /// Message content (text or caption)
    pub content: String,

    /// Message type
    pub message_type: MessageType,

    /// Media URL if applicable
    pub media_url: Option<String>,

    /// Reply to message ID
    pub reply_to: Option<String>,

    /// Unix timestamp
    pub timestamp: i64,

    /// Raw platform-specific data
    pub raw: Option<serde_json::Value>,
}

impl ChannelMessage {
    /// Create a simple text message
    pub fn text(
        channel: &str,
        sender_id: &str,
        chat_id: &str,
        content: &str,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            channel: channel.to_string(),
            sender_id: sender_id.to_string(),
            sender_name: None,
            chat_id: chat_id.to_string(),
            is_group: false,
            content: content.to_string(),
            message_type: MessageType::Text,
            media_url: None,
            reply_to: None,
            timestamp: chrono::Utc::now().timestamp(),
            raw: None,
        }
    }

    /// Normalize sender ID to i64 for unified storage
    pub fn sender_id_numeric(&self) -> i64 {
        // Try to parse as number, otherwise hash the string
        self.sender_id.parse::<i64>().unwrap_or_else(|_| {
            use std::hash::{Hash, Hasher};
            use std::collections::hash_map::DefaultHasher;
            let mut hasher = DefaultHasher::new();
            self.sender_id.hash(&mut hasher);
            hasher.finish() as i64
        })
    }

    /// Normalize chat ID to i64 for unified storage
    pub fn chat_id_numeric(&self) -> i64 {
        self.chat_id.parse::<i64>().unwrap_or_else(|_| {
            use std::hash::{Hash, Hasher};
            use std::collections::hash_map::DefaultHasher;
            let mut hasher = DefaultHasher::new();
            self.chat_id.hash(&mut hasher);
            hasher.finish() as i64
        })
    }
}

/// Response to send back through channel
#[derive(Debug, Clone)]
pub struct ChannelResponse {
    /// Target chat ID
    pub chat_id: String,

    /// Response content
    pub content: String,

    /// Reply to specific message
    pub reply_to: Option<String>,

    /// Inline buttons (platform-specific rendering)
    pub buttons: Vec<Vec<ResponseButton>>,

    /// Media attachment
    pub media: Option<MediaAttachment>,

    /// Parse mode (markdown, html, plain)
    pub parse_mode: ParseMode,
}

impl ChannelResponse {
    pub fn text(chat_id: &str, content: &str) -> Self {
        Self {
            chat_id: chat_id.to_string(),
            content: content.to_string(),
            reply_to: None,
            buttons: Vec::new(),
            media: None,
            parse_mode: ParseMode::Markdown,
        }
    }

    pub fn with_buttons(mut self, buttons: Vec<Vec<ResponseButton>>) -> Self {
        self.buttons = buttons;
        self
    }

    pub fn with_media(mut self, media: MediaAttachment) -> Self {
        self.media = Some(media);
        self
    }

    pub fn with_reply(mut self, reply_to: &str) -> Self {
        self.reply_to = Some(reply_to.to_string());
        self
    }
}

/// Button for inline keyboards
#[derive(Debug, Clone)]
pub struct ResponseButton {
    pub text: String,
    pub callback_data: Option<String>,
    pub url: Option<String>,
}

impl ResponseButton {
    pub fn callback(text: &str, data: &str) -> Self {
        Self {
            text: text.to_string(),
            callback_data: Some(data.to_string()),
            url: None,
        }
    }

    pub fn link(text: &str, url: &str) -> Self {
        Self {
            text: text.to_string(),
            callback_data: None,
            url: Some(url.to_string()),
        }
    }
}

/// Media attachment
#[derive(Debug, Clone)]
pub struct MediaAttachment {
    pub media_type: MessageType,
    pub url: Option<String>,
    pub data: Option<Vec<u8>>,
    pub filename: Option<String>,
    pub caption: Option<String>,
}

/// Parse mode for message formatting
#[derive(Debug, Clone, Copy, Default)]
pub enum ParseMode {
    #[default]
    Markdown,
    Html,
    Plain,
}

/// Callback data from button press
#[derive(Debug, Clone)]
pub struct CallbackData {
    pub id: String,
    pub chat_id: String,
    pub sender_id: String,
    pub data: String,
    pub message_id: Option<String>,
}

/// Channel trait - implement for each platform
#[async_trait]
pub trait Channel: Send + Sync {
    /// Channel name identifier
    fn name(&self) -> &str;

    /// Is the channel connected and ready?
    fn is_ready(&self) -> bool;

    /// Connect to the channel
    async fn connect(&mut self) -> Result<(), ChannelError>;

    /// Disconnect from the channel
    async fn disconnect(&mut self) -> Result<(), ChannelError>;

    /// Send a response
    async fn send(&self, response: ChannelResponse) -> Result<String, ChannelError>;

    /// Send typing indicator
    async fn send_typing(&self, chat_id: &str) -> Result<(), ChannelError>;

    /// Edit a previously sent message
    async fn edit(&self, chat_id: &str, message_id: &str, content: &str) -> Result<(), ChannelError>;

    /// Delete a message
    async fn delete(&self, chat_id: &str, message_id: &str) -> Result<(), ChannelError>;

    /// Answer a callback (button press)
    async fn answer_callback(&self, callback_id: &str, text: Option<&str>) -> Result<(), ChannelError>;

    /// Broadcast to all active chats
    async fn broadcast(&self, message: &str) -> Result<(), ChannelError>;

    /// Get channel-specific configuration
    fn config(&self) -> serde_json::Value;
}

/// Sender trait for simplified sending
#[async_trait]
pub trait ChannelSender: Send + Sync {
    async fn send_text(&self, chat_id: &str, text: &str) -> Result<String, ChannelError>;
    async fn send_image(&self, chat_id: &str, url: &str, caption: Option<&str>) -> Result<String, ChannelError>;
    async fn send_document(&self, chat_id: &str, url: &str, filename: &str) -> Result<String, ChannelError>;
}

impl fmt::Display for MessageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MessageType::Text => write!(f, "text"),
            MessageType::Image => write!(f, "image"),
            MessageType::Audio => write!(f, "audio"),
            MessageType::Video => write!(f, "video"),
            MessageType::Document => write!(f, "document"),
            MessageType::Location => write!(f, "location"),
            MessageType::Contact => write!(f, "contact"),
            MessageType::Sticker => write!(f, "sticker"),
            MessageType::Voice => write!(f, "voice"),
            MessageType::Unknown => write!(f, "unknown"),
        }
    }
}
