//! Bridge Module
//!
//! gRPC streaming bridge for remote Claude Code execution with TLS.
//! AR server runs the bridge service, Hetzner clients connect to execute tasks.

pub mod proto;
pub mod grpc_server;
pub mod grpc_client;
pub mod types;

pub use grpc_server::{GrpcBridgeServer, GrpcBridgeConfig};
pub use grpc_client::{GrpcBridgeClient, GrpcBridgeClientConfig, ExecuteResult};
pub use types::ClaudeCliOutput;
