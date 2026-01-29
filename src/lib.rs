//! ClaudeBot MCP Server
//!
//! All-Rust Model Context Protocol server for Claude Code integration.
//!
//! # Features
//!
//! - **MCP Protocol**: JSON-RPC 2.0 over stdio
//! - **Intelligent Routing**: Llama-based complexity classification (Haiku/Sonnet/Opus)
//! - **Prompt Caching**: Anthropic cache_control for 90% cost reduction
//! - **Response Caching**: Context-aware SHA256 caching
//! - **Hybrid Memory**: Vector + Graph for semantic search
//! - **Graph Memory**: Entity extraction and relationship tracking
//! - **Development Circle**: 5-persona quality pipeline
//! - **Metrics**: Cost tracking and performance monitoring
//!
//! # Architecture
//!
//! ```text
//! Claude Code ──► MCP Protocol ──► ClaudeBot ──► Claude API
//!                   (stdio)           │
//!                                     ├── Router (Ollama/Llama)
//!                                     ├── Cache (Moka + Redis)
//!                                     ├── Memory (SQLite + FTS5)
//!                                     ├── Graph (Entities + Relations)
//!                                     ├── Circle (5 Personas)
//!                                     ├── Metrics (Cost + Latency)
//!                                     └── Tools (20+ MCP tools)
//! ```

pub mod auto_review;
pub mod bridge;
pub mod cache;
pub mod circle;
pub mod claude;
pub mod config;
pub mod conversation;
pub mod embeddings;
pub mod graph;
pub mod lifecycle;
pub mod llama_worker;
pub mod mcp;
pub mod memory;
pub mod metrics;
pub mod permissions;
pub mod router;
pub mod telegram;
pub mod tokenizer;
pub mod tools;
pub mod usage;

#[cfg(test)]
mod telegram_tests;

pub use cache::ResponseCache;
pub use circle::{Circle, PipelineMode, PipelineResult};
pub use claude::ClaudeClient;
pub use config::Config;
pub use conversation::{ConversationStore, ConversationMessage, ConversationSummary};
pub use embeddings::{EmbeddingStore, EmbeddingConfig, VectorIndex, embedding_to_bytes, embedding_from_bytes};
pub use graph::GraphStore;
pub use memory::{MemoryStore, MemoryEntry, ScoredMemory, SearchResult, MemoryStats, EmbeddingStats};
pub use lifecycle::{LifecycleManager, LifecycleConfig, State as LifecycleState};
pub use llama_worker::{LlamaWorker, LlamaWorkerConfig, QueryComplexity};
pub use mcp::{McpRequest, McpResponse, McpServer};
pub use metrics::MetricsCollector;
pub use router::{ModelHint, RouteResult, Target, TaskRouter};
pub use tokenizer::{BudgetCheck, TokenCounter};
pub use bridge::{GrpcBridgeServer, GrpcBridgeClient, GrpcBridgeConfig, GrpcBridgeClientConfig, ExecuteResult};
