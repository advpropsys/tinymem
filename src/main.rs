mod models;
mod server;
mod store;
mod tui;
mod mcp;
mod mcp_tools;

use anyhow::Result;
use clap::Parser;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(name = "tinymem", about = "AI Agent Coordination Framework")]
struct Args {
    /// Redis URL
    #[arg(long, default_value = "redis://127.0.0.1:6379", env = "TINYMEM_REDIS")]
    redis: String,

    /// Server port
    #[arg(long, default_value = "3000", env = "TINYMEM_PORT")]
    port: u16,

    /// Auth token (empty = no auth)
    #[arg(long, default_value = "", env = "TINYMEM_TOKEN")]
    token: String,

    /// Headless mode (no TUI, server only)
    #[arg(long)]
    headless: bool,

    /// MCP server mode (stdio)
    #[arg(long)]
    mcp: bool,

    /// Host for MCP client to connect to
    #[arg(long, default_value = "localhost", env = "TINYMEM_HOST")]
    host: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // MCP mode: run as stdio MCP server (client to main tinymem)
    if args.mcp {
        mcp::run(&args.host, args.port, &args.token);
        return Ok(());
    }

    let store = store::Store::new(&args.redis).await?;
    let (tui_tx, tui_rx) = mpsc::channel(100);

    let server_store = store.clone();
    let token = args.token.clone();
    let port = args.port;
    let server_handle = tokio::spawn(async move {
        server::run(server_store, token, tui_tx, port).await
    });

    // Spawn cleanup task - mark sessions inactive after 2 minutes of no activity
    let cleanup_store = store.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            if let Ok(cleaned) = cleanup_store.cleanup_stale(120).await {
                let _ = cleaned; // silence unused warning
            }
        }
    });

    if args.headless {
        eprintln!("Running in headless mode (no TUI)");
        server_handle.await??;
    } else {
        let mut terminal = ratatui::init();
        let mut app = tui::App::new(store, tui_rx);
        let result = app.run(&mut terminal).await;
        ratatui::restore();
        result?;
    }

    Ok(())
}
