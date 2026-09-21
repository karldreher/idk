use clap::Parser;

#[derive(Parser)]
#[command(name = "idk", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
