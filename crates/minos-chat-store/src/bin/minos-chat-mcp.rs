use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "minos-chat-mcp",
    about = "Expose Minos chat room history over MCP stdio"
)]
struct Args {
    #[arg(long)]
    db_path: Option<PathBuf>,

    #[arg(long)]
    default_room_id: Option<String>,

    #[arg(long)]
    source_agent: Option<String>,

    #[arg(long)]
    disable_read_chat: bool,

    #[arg(long)]
    disable_mention_agent: bool,

    #[arg(long)]
    disable_mention_user: bool,

    #[arg(long)]
    allow_any_room: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let source_agent = args
        .source_agent
        .as_deref()
        .map(parse_agent_name)
        .transpose()?;
    minos_chat_store::mcp::serve_stdio_with_config(
        args.db_path,
        minos_chat_store::mcp::ChatMcpServerConfig {
            default_room_id: args.default_room_id,
            source_agent,
            permissions: minos_chat_store::mcp::ChatMcpToolPermissions {
                read_chat: !args.disable_read_chat,
                mention_agent: !args.disable_mention_agent,
                mention_user: !args.disable_mention_user,
                allow_any_room: args.allow_any_room,
            },
        },
    )
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
