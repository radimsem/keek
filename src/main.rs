use clap::Parser;

#[derive(Parser)]
#[command(
    version,
    about = "Verify a change on a sandboxed nested Hyprland desktop"
)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
