use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "minos-mcp",
    about = "Expose Minos features over MCP stdio, proxied to the Minos main process via Unix socket"
)]
struct Args {
    #[arg(long)]
    socket_path: PathBuf,

    #[arg(long)]
    db_path: PathBuf,

    #[arg(long)]
    room_id: String,

    #[arg(long)]
    source_agent: Option<String>,

    #[arg(long)]
    disable_read_chat: bool,

    #[arg(long)]
    disable_mention_agent: bool,

    #[arg(long)]
    disable_mention_user: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let source_agent = args
        .source_agent
        .as_deref()
        .map(parse_agent_name)
        .transpose()?;
    minos_chat_store::mcp_server::serve_stdio(minos_chat_store::mcp_server::McpServerConfig {
        socket_path: args.socket_path,
        db_path: args.db_path,
        room_id: args.room_id,
        source_agent,
        permissions: minos_chat_store::mcp_server::McpToolPermissions {
            read_chat: !args.disable_read_chat,
            mention_agent: !args.disable_mention_agent,
            mention_user: !args.disable_mention_user,
        },
    })
    .await
}

fn parse_agent_name(value: &str) -> Result<minos_domain::AgentName> {
    let normalized = value.trim().to_ascii_lowercase();
    minos_domain::AgentName::all()
        .iter()
        .copied()
        .find(|agent| agent.bin_name() == normalized.as_str())
        .ok_or_else(|| anyhow::anyhow!("unknown agent: {value}"))
}
