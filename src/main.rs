// src/main.rs (or examples/nostr_jobs_mcp.rs)

use rmcp::transport::sse_server::SseServer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use jobmcp::NostrJobsServer;

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

    println!("🚀 Starting Nostr Jobs MCP Server");
    println!("📡 Binding to: {}", BIND_ADDRESS);
    println!("🔗 MCP endpoint: http://{}/sse", BIND_ADDRESS);
    println!();
    println!("💡 Connecting to Nostr relays...");
    
    // Create the server instance first (this is async)
    let server = NostrJobsServer::new().await;
    
    // Clone it for the service provider
    let server_clone = server.clone();
    
    // This spawns background tasks to handle the server
    let ct = SseServer::serve(BIND_ADDRESS.parse()?)
        .await?
        .with_service_directly(move || server_clone.clone());
    
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
    
    // Main task blocks here, waiting for Ctrl+C
    tokio::signal::ctrl_c().await?;
    
    println!("\n🛑 Shutting down server...");
    // Signal background tasks to stop
    ct.cancel();
    
    println!("✅ Server stopped");
    Ok(())
}
