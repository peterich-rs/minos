// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Desktop entrypoint.
//!
//! When the embedded daemon injects teamwork MCP, OpenCode/Grok spawn
//! `current_exe __minos-teamwork-mcp ...` (same pattern as minos-daemon).
//! Handle that hidden mode **before** starting Tauri so agents get `minos_teamwork`.

use std::path::PathBuf;

use clap::{Args, Parser};
use minos_domain::AgentName;

const TEAMWORK_MCP_SIDECAR: &str = "__minos-teamwork-mcp";

#[derive(Parser, Debug)]
#[command(
    name = "minos-desktop",
    disable_help_flag = true,
    disable_version_flag = true
)]
struct SidecarCli {
    #[command(subcommand)]
    command: Option<SidecarCommand>,
}

#[derive(clap::Subcommand, Debug)]
enum SidecarCommand {
    #[command(name = "__minos-teamwork-mcp", hide = true)]
    MinosTeamworkMcp(McpSidecarArgs),
}

#[derive(Args, Debug)]
struct McpSidecarArgs {
    #[arg(long)]
    socket_path: PathBuf,

    #[arg(long)]
    conversation_id: String,

    #[arg(long)]
    source_agent: Option<String>,

    /// Minos session id of the invoking agent (manager injects this for bound MCP).
    /// Accepts the historical alias used in older docs/configs.
    #[arg(long = "source-session-id", alias = "source-thread-id")]
    source_session_id: Option<String>,

    #[arg(long)]
    disable_list_conversation_messages: bool,

    #[arg(long)]
    disable_list_conversation_roster: bool,

    #[arg(long)]
    disable_delegate_to_agent: bool,

    #[arg(long)]
    disable_get_delegation_status: bool,

    #[arg(long)]
    disable_wait_delegation: bool,

    #[arg(long)]
    disable_cancel_delegation: bool,

    #[arg(long)]
    disable_post_conversation_update: bool,

    #[arg(long)]
    disable_post_git_update: bool,
}

impl McpSidecarArgs {
    async fn serve(self) -> Result<(), Box<dyn std::error::Error>> {
        let source_agent = self
            .source_agent
            .as_deref()
            .map(parse_agent_name)
            .transpose()?;
        minos_chat_store::mcp_server::serve_stdio(minos_chat_store::mcp_server::McpServerConfig {
            socket_path: self.socket_path,
            conversation_id: self.conversation_id,
            source_agent,
            source_session_id: self.source_session_id,
            permissions: minos_chat_store::mcp_server::McpToolPermissions {
                list_conversation_messages: !self.disable_list_conversation_messages,
                list_conversation_roster: !self.disable_list_conversation_roster,
                delegate_to_agent: !self.disable_delegate_to_agent,
                get_delegation_status: !self.disable_get_delegation_status,
                wait_delegation: !self.disable_wait_delegation,
                cancel_delegation: !self.disable_cancel_delegation,
                post_conversation_update: !self.disable_post_conversation_update,
                post_git_update: !self.disable_post_git_update,
                react_to_message: true,
            },
        })
        .await?;
        Ok(())
    }
}

fn parse_agent_name(s: &str) -> Result<AgentName, String> {
    match s.to_lowercase().as_str() {
        "codex" => Ok(AgentName::Codex),
        "claude" => Ok(AgentName::Claude),
        "gemini" => Ok(AgentName::Gemini),
        "opencode" => Ok(AgentName::Opencode),
        "grok" => Ok(AgentName::Grok),
        _ => Err(format!(
            "unknown agent: {s} (expected codex|claude|gemini|opencode|grok)"
        )),
    }
}

fn main() {
    // Fast path: only enter clap when the hidden MCP subcommand is present so
    // normal Tauri launches stay free of argv parsing surprises.
    let wants_mcp = std::env::args_os()
        .skip(1)
        .any(|a| a == TEAMWORK_MCP_SIDECAR);
    if wants_mcp {
        let cli = SidecarCli::parse();
        match cli.command {
            Some(SidecarCommand::MinosTeamworkMcp(args)) => {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                if let Err(err) = rt.block_on(args.serve()) {
                    eprintln!("minos-desktop teamwork MCP failed: {err}");
                    std::process::exit(1);
                }
                return;
            }
            None => {
                eprintln!("minos-desktop: expected subcommand {TEAMWORK_MCP_SIDECAR}");
                std::process::exit(2);
            }
        }
    }

    minos_desktop_lib::run()
}
