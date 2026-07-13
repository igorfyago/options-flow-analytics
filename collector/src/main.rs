mod analytics;
mod db;
mod greeks;
mod models;
mod provider;

use anyhow::Result;
use std::time::Duration;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    let database_url = env_or(
        "DATABASE_URL",
        "host=localhost user=postgres password=postgres dbname=options",
    );
    let tickers: Vec<String> = env_or("TICKERS", "SPY")
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect();
    let interval = Duration::from_secs(env_or("INTERVAL_SECS", "60").parse().unwrap_or(60));
    let retention_days: f64 = env_or("RETENTION_DAYS", "30").parse().unwrap_or(30.0);

    println!("collector starting: tickers={tickers:?} interval={interval:?} retention={retention_days}d");

    let client = db::connect(&database_url).await?;
    let mut provider = provider::from_env();
    let mut cycles: u64 = 0;

    loop {
        for ticker in &tickers {
            match provider.fetch_chain(ticker) {
                Ok(chain) => {
                    let a = analytics::compute(&chain);
                    match db::insert_snapshot(&client, &a).await {
                        Ok(()) => println!(
                            "{} spot={:.2} netGEX={:.2}M flip={} regime={} light={}",
                            a.ticker,
                            a.spot,
                            a.net_gex_total / 1e6,
                            a.gamma_flip.map_or("n/a".into(), |f| format!("{f:.1}")),
                            a.regime,
                            a.traffic_light,
                        ),
                        Err(e) => eprintln!("{ticker}: insert failed: {e}"),
                    }
                }
                Err(e) => eprintln!("{ticker}: fetch failed: {e}"),
            }
        }

        cycles += 1;
        if cycles % 60 == 0 {
            match db::prune(&client, retention_days).await {
                Ok(n) if n > 0 => println!("pruned {n} snapshots older than {retention_days}d"),
                Ok(_) => {}
                Err(e) => eprintln!("prune failed: {e}"),
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = tokio::signal::ctrl_c() => {
                println!("shutting down");
                return Ok(());
            }
        }
    }
}
