use clap::Parser;
use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

use kicad_mcp::mcp::KicadMcp;

#[derive(Parser, Debug)]
#[command(
    name = "kicad-mcp",
    about = "Drive a running KiCad PCB editor from Cursor over MCP"
)]
struct Args {
    /// Enable download/place/remove/outline/nets/copper/ripup/save. Without this, every write tool refuses.
    #[arg(long)]
    allow_ai_write: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // MCP speaks on stdout — log to stderr only.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("kicad_mcp=info".parse()?))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let args = Args::parse();
    eprintln!(
        "kicad-mcp: stdio MCP, write tools {}",
        if args.allow_ai_write {
            "ENABLED"
        } else {
            "disabled — pass --allow-ai-write"
        }
    );
    eprintln!("kicad-mcp: KiCad must be running with Preferences → Plugins → Enable IPC API");

    let server = KicadMcp::new(args.allow_ai_write);
    let running = server.serve(rmcp::transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}
