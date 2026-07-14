//! Market-data providers. Default is the real CBOE delayed feed (free, no
//! key). `PROVIDER=synthetic` selects the generator, which produces a
//! realistic chain (IV smile, OI concentration) for offline/demo runs.

use crate::models::{ChainSnapshot, OptionContract, OptionKind};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use rand::Rng;

#[async_trait]
pub trait MarketDataProvider: Send {
    async fn fetch_chain(&mut self, ticker: &str) -> Result<ChainSnapshot>;
}

/// Synthetic chain generator: spot follows a slow random walk per ticker,
/// strikes span ±15% in 1% steps, IV has a put skew, OI clusters on round
/// strikes. Good enough to exercise every downstream computation honestly.
pub struct SyntheticProvider {
    base_spot: f64,
    drift_state: f64,
}

impl SyntheticProvider {
    pub fn new(base_spot: f64) -> Self {
        Self { base_spot, drift_state: 0.0 }
    }
}

#[async_trait]
impl MarketDataProvider for SyntheticProvider {
    async fn fetch_chain(&mut self, ticker: &str) -> Result<ChainSnapshot> {
        let (spot, atm_iv, vix) = {
            let mut rng = rand::thread_rng();
            self.drift_state = 0.95 * self.drift_state + rng.gen_range(-0.4..0.4);
            (
                self.base_spot * (1.0 + self.drift_state / 100.0),
                rng.gen_range(0.16..0.24),
                rng.gen_range(13.0..22.0),
            )
        };

        let expiry = (Utc::now() + Duration::days(14)).date_naive();
        let mut contracts = Vec::new();
        let mut rng = rand::thread_rng();

        let mut pct = -15.0_f64;
        while pct <= 15.0 {
            let strike = (spot * (1.0 + pct / 100.0)).round();
            // Put skew: IV rises as strikes fall below spot.
            let skew = if pct < 0.0 { -pct * 0.004 } else { pct * 0.001 };
            let iv = (atm_iv + skew).max(0.05);

            // OI: dense near the money, heavier on round-number strikes.
            let moneyness_penalty = (-(pct * pct) / 60.0).exp();
            let round_bonus = if strike % 5.0 == 0.0 { 2.2 } else { 1.0 };
            let base_oi = 12_000.0 * moneyness_penalty * round_bonus;

            for kind in [OptionKind::Call, OptionKind::Put] {
                let side_bias = match kind {
                    OptionKind::Call if pct > 0.0 => 1.25, // calls stack above spot
                    OptionKind::Put if pct < 0.0 => 1.35,  // puts stack below
                    _ => 1.0,
                };
                contracts.push(OptionContract {
                    kind,
                    strike,
                    expiry,
                    open_interest: (base_oi * side_bias * rng.gen_range(0.7..1.3)).round(),
                    volume: (base_oi * 0.15 * rng.gen_range(0.2..1.8)).round(),
                    implied_vol: iv * rng.gen_range(0.97..1.03),
                });
            }
            pct += 1.0;
        }

        Ok(ChainSnapshot {
            ticker: ticker.to_string(),
            spot,
            vix: Some(vix),
            taken_at: Utc::now(),
            contracts,
        })
    }
}

pub fn from_env() -> Box<dyn MarketDataProvider> {
    match std::env::var("PROVIDER").as_deref() {
        Ok("synthetic") => Box::new(SyntheticProvider::new(
            std::env::var("SYNTHETIC_SPOT").ok().and_then(|s| s.parse().ok()).unwrap_or(500.0),
        )),
        Ok("cboe") | Err(_) => match crate::cboe::CboeProvider::new() {
            Ok(p) => Box::new(p),
            Err(e) => {
                eprintln!("cboe provider init failed ({e}); falling back to synthetic");
                Box::new(SyntheticProvider::new(500.0))
            }
        },
        Ok(other) => {
            eprintln!("unknown provider '{other}'; falling back to synthetic");
            Box::new(SyntheticProvider::new(500.0))
        }
    }
}
