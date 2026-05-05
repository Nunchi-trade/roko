//! Quick non-interactive bench demo preview.
//! Run with: cargo run -p roko-cli --example bench_autoplay

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workdir = std::path::PathBuf::from(".");
    roko_cli::bench_demo::run_bench_demo(&workdir, false).await?;
    Ok(())
}
