use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "minos-teamwork-mcp",
    about = "Expose Minos features over MCP stdio, proxied to the Minos main process via Unix socket"
)]
struct Args {
    #[arg(long)]
    socket_path: PathBuf,

    #[arg(long)]
    room_id: String,

    #[arg(long)]
    source_agent: Option<String>,

    #[arg(long)]
    disable_list_room_messages: bool,

    #[arg(long)]
    disable_delegate_to_agent: bool,

    #[arg(long)]
    disable_get_delegation_status: bool,

    #[arg(long)]
    disable_cancel_delegation: bool,

    #[arg(long)]
    disable_ask_user_question: bool,

    #[arg(long)]
    disable_check_user_feedback: bool,

    #[arg(long)]
    disable_post_room_update: bool,

    #[arg(long)]
    disable_react_to_message: bool,
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
        room_id: args.room_id,
        source_agent,
        permissions: minos_chat_store::mcp_server::McpToolPermissions {
            list_room_messages: !args.disable_list_room_messages,
            delegate_to_agent: !args.disable_delegate_to_agent,
            get_delegation_status: !args.disable_get_delegation_status,
            cancel_delegation: !args.disable_cancel_delegation,
            ask_user_question: !args.disable_ask_user_question,
            check_user_feedback: !args.disable_check_user_feedback,
            post_room_update: !args.disable_post_room_update,
            react_to_message: !args.disable_react_to_message,
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
