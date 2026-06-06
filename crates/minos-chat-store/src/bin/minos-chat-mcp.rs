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
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    minos_chat_store::mcp::serve_stdio(args.db_path, args.default_room_id).await
}
