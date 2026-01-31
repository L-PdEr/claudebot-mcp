//! Browser Automation Module
//!
//! Provides tools for web interaction:
//! - Screenshot capture
//! - Element clicking
//! - Form filling
//! - Text extraction
//! - Page navigation
//!
//! Uses headless Chrome via chromiumoxide or falls back to simple HTTP.

pub mod tools;
pub mod pool;

pub use tools::{
    browser_screenshot_tool,
    browser_click_tool,
    browser_fill_tool,
    browser_extract_tool,
    browser_navigate_tool,
};
pub use pool::{BrowserPool, BrowserConfig, BrowserSession};
