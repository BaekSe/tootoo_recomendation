use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "tootoo_worker")]
struct Args {
    /// Market as-of date (YYYY-MM-DD). Defaults to today (UTC) for now.
    #[arg(long)]
    as_of_date: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let as_of_date = match args.as_of_date {
        Some(s) => chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d")?,
        None => chrono::Utc::now().date_naive(),
    };

    tracing::info!(%as_of_date, "worker placeholder: EOD run");
    Ok(())
}
