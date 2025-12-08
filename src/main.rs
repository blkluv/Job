// src/main.rs - Updated for HTTP Streamable Transport

use rmcp::transport::streamable_http_server::{
    StreamableHttpService,
    session::local::LocalSessionManager
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use jobmcp::NostrJobsServer;
use std::net::SocketAddr;

const BIND_ADDRESS: &str = "127.0.0.1:8000";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,jobmcp=debug".to_string().into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    println!("🚀 Starting Nostr Jobs MCP Server (HTTP Streamable)");
    println!("📡 Binding to: {}", BIND_ADDRESS);
    println!("🔗 MCP endpoint: http://{}/mcp", BIND_ADDRESS);
    println!();
    println!("💡 Connecting to Nostr relays...");
    
    // Create the HTTP service with factory closure that returns Result<NostrJobsServer, io::Error>
    // The factory is called for each new session
    // Note: Since NostrJobsServer::new() is async, we need to block on it here
    // This means the server initialization happens synchronously during session creation
    let service = StreamableHttpService::new(
        || {
            // Block on the async initialization
            // This works because we're in a tokio runtime context
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    Ok(NostrJobsServer::new().await)
                })
            })
        },
        LocalSessionManager::default().into(),
        Default::default(),
    );

    // Create axum router and mount the MCP service at /mcp
    let router = axum::Router::new()
        .nest_service("/mcp", service);

    // Parse the bind address
    let addr: SocketAddr = BIND_ADDRESS.parse()?;
    
    // Create the TCP listener
    let listener = tokio::net::TcpListener::bind(addr).await?;
    
    println!("✅ Server is running!");
    println!("📋 Available tools:");
    println!("   • search_jobs - Search for job listings");
    println!("   • get_job_details - Get detailed job info");
    println!("   • list_relays - Show connected relays");
    println!("   • get_stats - Job market statistics");
    println!();
    println!("📚 Available resources:");
    println!("   • jobs://latest - Latest job listings");
    println!("   • jobs://stats - Job market stats");
    println!();
    println!("Press Ctrl+C to stop the server...");
    println!();

    // Serve with graceful shutdown
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to listen for ctrl-c");
            println!("\n🛑 Shutting down server...");
        })
        .await?;
    
    println!("✅ Server stopped");
    Ok(())
}

