//! Multi-Channel Communication Framework
//!
//! Unified abstraction for all messaging channels:
//! - Telegram (implemented)
//! - WhatsApp (Twilio/Baileys)
//! - Discord (Bot API)
//! - Slack (Bolt SDK)
//! - WebChat (WebSocket)
//!
//! Each channel implements the `Channel` trait for unified message handling.

pub mod traits;
pub mod whatsapp;
pub mod discord;
pub mod webchat;

pub use traits::{ChannelMessage, MessageType, ChannelError, ChannelResponse, ResponseButton, ParseMode};
pub use whatsapp::{WhatsAppChannel, WhatsAppConfig};
pub use discord::{DiscordChannel, DiscordConfig};
pub use webchat::{WebChatChannel, WebChatConfig};

use tokio::sync::RwLock;

/// Supported channel types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelType {
    Telegram,
    WhatsApp,
    Discord,
    Slack,
    WebChat,
}

impl ChannelType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
            Self::WhatsApp => "whatsapp",
            Self::Discord => "discord",
            Self::Slack => "slack",
            Self::WebChat => "webchat",
        }
    }
}

/// Channel registry for tracking active channels
pub struct ChannelRegistry {
    /// Active WhatsApp channels
    whatsapp: RwLock<Option<WhatsAppChannel>>,
    /// Active Discord channels
    discord: RwLock<Option<DiscordChannel>>,
    /// Active WebChat channels
    webchat: RwLock<Option<WebChatChannel>>,
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self {
            whatsapp: RwLock::new(None),
            discord: RwLock::new(None),
            webchat: RwLock::new(None),
        }
    }

    /// Register WhatsApp channel
    pub async fn set_whatsapp(&self, channel: WhatsAppChannel) {
        *self.whatsapp.write().await = Some(channel);
    }

    /// Register Discord channel
    pub async fn set_discord(&self, channel: DiscordChannel) {
        *self.discord.write().await = Some(channel);
    }

    /// Register WebChat channel
    pub async fn set_webchat(&self, channel: WebChatChannel) {
        *self.webchat.write().await = Some(channel);
    }

    /// List active channels
    pub async fn list_active(&self) -> Vec<ChannelType> {
        let mut active = Vec::new();
        if self.whatsapp.read().await.is_some() {
            active.push(ChannelType::WhatsApp);
        }
        if self.discord.read().await.is_some() {
            active.push(ChannelType::Discord);
        }
        if self.webchat.read().await.is_some() {
            active.push(ChannelType::WebChat);
        }
        active
    }
}

impl Default for ChannelRegistry {
    fn default() -> Self {
        Self::new()
    }
}
