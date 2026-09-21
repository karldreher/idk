use clap::Parser;

#[derive(Parser)]
#[command(name = "idk", version, about)]
struct Cli {}

#[tokio::main]
async fn main() {
    let _cli = Cli::parse();
}
