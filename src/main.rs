//! ClaudeBot MCP Server - Entry Point
//!
//! Modes:
//! - Default: MCP server over stdio
//! - --telegram / -t: Telegram bot mode
//! - --grpc-server / -g: gRPC bridge server mode

use claudebot_mcp::{Config, McpServer, GrpcBridgeServer};
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load environment
    dotenvy::dotenv().ok();

    // Parse args
    let args: Vec<String> = std::env::args().collect();
    let telegram_mode = args.iter().any(|a| a == "--telegram" || a == "-t");
    let grpc_server_mode = args.iter().any(|a| a == "--grpc-server" || a == "-g");
    let help_mode = args.iter().any(|a| a == "--help" || a == "-h");

    if help_mode {
        println!("ClaudeBot MCP Server v{}", env!("CARGO_PKG_VERSION"));
        println!();
        println!("Usage: claudebot-mcp [OPTIONS]");
        println!();
        println!("Options:");
        println!("  --telegram, -t     Run as Telegram bot");
        println!("  --grpc-server, -g  Run as gRPC bridge server");
        println!("  --help, -h         Show this help");
        println!();
        println!("Default: Run as MCP server (stdio)");
        println!();
        println!("Environment variables:");
        println!("  TELOXIDE_TOKEN       Telegram bot token");
        println!("  ANTHROPIC_API_KEY    Claude API key");
        println!("  BRIDGE_API_KEY       gRPC bridge authentication");
        println!("  BRIDGE_GRPC_PORT     gRPC server port (default: 9998)");
        println!("  BRIDGE_GRPC_URL      gRPC server URL (for client)");
        return Ok(());
    }

    // Setup logging based on mode
    let log_level = std::env::var("RUST_LOG")
        .map(|s| match s.to_lowercase().as_str() {
            "trace" => Level::TRACE,
            "debug" => Level::DEBUG,
            "warn" => Level::WARN,
            "error" => Level::ERROR,
            _ => Level::INFO,
        })
        .unwrap_or(if telegram_mode || grpc_server_mode { Level::DEBUG } else { Level::INFO });

    if telegram_mode || grpc_server_mode {
        // Interactive modes - log to stdout with colors
        let subscriber = FmtSubscriber::builder()
            .with_max_level(log_level)
            .with_ansi(true)
            .finish();
        tracing::subscriber::set_global_default(subscriber)?;
    } else {
        // MCP mode - log to stderr as JSON
        let subscriber = FmtSubscriber::builder()
            .with_max_level(log_level)
            .with_writer(std::io::stderr)
            .with_ansi(false)
            .json()
            .finish();
        tracing::subscriber::set_global_default(subscriber)?;
    }

    if grpc_server_mode {
        info!("ClaudeBot gRPC Bridge Server v{}", env!("CARGO_PKG_VERSION"));

        let server = GrpcBridgeServer::from_env()?;
        info!("Starting on port {}", server.port());
        server.run().await?;
    } else if telegram_mode {
        info!("ClaudeBot Telegram Bot v{}", env!("CARGO_PKG_VERSION"));

        claudebot_mcp::telegram::run_telegram_bot().await?;
    } else {
        info!("ClaudeBot MCP Server v{}", env!("CARGO_PKG_VERSION"));

        let config = Config::from_env()?;
        let server = McpServer::new(config).await?;
        server.run().await?;
    }

    Ok(())
}
