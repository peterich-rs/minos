use clap::Parser;

mod backend;
mod event;

#[derive(Parser, Debug)]
#[command(name = "minos-tui", about = "Minos Agent TUI - local debug console")]
struct Cli {
    #[arg(short, long)]
    agent: Option<String>,

    #[arg(short, long, default_value = ".")]
    workspace: std::path::PathBuf,

    #[arg(long)]
    readonly: bool,

    #[arg(long)]
    max_instances: Option<usize>,
}

fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    println!("minos-tui: not yet implemented");
    Ok(())
}
