use clap::Parser;

#[tokio::main]
async fn main() -> miette::Result<()> {
    miette::set_panic_hook();
    let cli = viri::cli::Cli::parse();
    viri::cli::run(cli).await
}
