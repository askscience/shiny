//! `shiny-iroh-client` — a local HTTP proxy that tunnels a browser to a Shiny
//! server over Iroh.
//!
//!   shiny-iroh-client --ticket <hex> [--listen 127.0.0.1:8080]

use std::error::Error;
use std::net::SocketAddr;

use shiny_iroh_client::{parse_ticket, serve};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let mut ticket: Option<String> = None;
    let mut listen = "127.0.0.1:8080".to_string();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ticket" | "-t" => {
                ticket = args.get(i + 1).cloned();
                i += 2;
            }
            "--listen" | "-l" => {
                if let Some(v) = args.get(i + 1) {
                    listen = v.clone();
                }
                i += 2;
            }
            other => {
                ticket = Some(other.to_string());
                i += 1;
            }
        }
    }

    let ticket = ticket.ok_or(
        "usage: shiny-iroh-client --ticket <shiny-iroh://… link> [--listen host:port]",
    )?;
    parse_ticket(&ticket).map_err(|e| format!("invalid ticket: {e}"))?;

    let listen: SocketAddr = listen.parse()?;
    let listener = TcpListener::bind(listen).await?;
    tracing::info!("listening on http://{listen} — open it in a browser");
    serve(listener, &ticket).await?;
    Ok(())
}
