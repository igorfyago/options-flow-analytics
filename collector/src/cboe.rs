//! Real market data from CBOE's free delayed-quotes CDN (15-min delayed,
//! no API key): https://cdn.cboe.com/api/global/delayed_quotes/options/{SYM}.json
//! Each option row carries IV and open interest, which is all the analytics
//! layer needs; we compute our own greeks from IV so the pipeline is
//! provider-agnostic (CBOE's own delta/gamma make a handy cross-check).

use crate::models::{ChainSnapshot, OptionContract, OptionKind};
use crate::provider::MarketDataProvider;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use serde::Deserialize;

const BASE: &str = "https://cdn.cboe.com/api/global/delayed_quotes/options";

#[derive(Deserialize)]
struct CboeResponse {
    data: CboeData,
}

#[derive(Deserialize)]
struct CboeData {
    current_price: f64,
    #[serde(default)]
    options: Vec<CboeOption>,
}

#[derive(Deserialize)]
struct CboeOption {
    option: String,
    #[serde(default)]
    iv: f64,
    #[serde(default)]
    open_interest: f64,
    #[serde(default)]
    volume: f64,
}

/// Parsed OCC-style symbol, e.g. "SPY260930P00696000":
/// root + YYMMDD + C/P + strike*1000 (8 digits).
pub fn parse_occ(sym: &str) -> Option<(OptionKind, NaiveDate, f64)> {
    if sym.len() < 16 {
        return None;
    }
    let strike: f64 = sym.get(sym.len() - 8..)?.parse::<f64>().ok()? / 1000.0;
    let kind = match sym.as_bytes()[sym.len() - 9] {
        b'C' => OptionKind::Call,
        b'P' => OptionKind::Put,
        _ => return None,
    };
    let date = sym.get(sym.len() - 15..sym.len() - 9)?;
    let (yy, mm, dd) = (
        date.get(0..2)?.parse::<i32>().ok()?,
        date.get(2..4)?.parse::<u32>().ok()?,
        date.get(4..6)?.parse::<u32>().ok()?,
    );
    let expiry = NaiveDate::from_ymd_opt(2000 + yy, mm, dd)?;
    Some((kind, expiry, strike))
}

pub struct CboeProvider {
    client: reqwest::Client,
}

impl CboeProvider {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent("options-flow-analytics/0.1 (personal research)")
            .build()?;
        Ok(Self { client })
    }

    async fn get(&self, symbol: &str) -> Result<CboeResponse> {
        let url = format!("{BASE}/{symbol}.json");
        let resp = self.client.get(&url).send().await.context("cboe request")?;
        if !resp.status().is_success() {
            return Err(anyhow!("cboe {symbol}: HTTP {}", resp.status()));
        }
        Ok(resp.json().await.context("cboe json")?)
    }
}

#[async_trait]
impl MarketDataProvider for CboeProvider {
    async fn fetch_chain(&mut self, ticker: &str) -> Result<ChainSnapshot> {
        let body = self.get(ticker).await?;
        let spot = body.data.current_price;
        let now = Utc::now();

        // Parse every contract, then keep the nearest expiry still ALIVE:
        // same-day contracts count until the 16:00 ET close, because the
        // 0DTE chain is the tenor the desk actually trades. The old
        // `num_days() >= 1` filter (whole days, UTC) never stored it - every
        // session captured tomorrow's chain, Fridays a 3-day weekend one -
        // and the roll to the next expiry happened a whole session early.
        let mut parsed: Vec<(NaiveDate, OptionContract)> = Vec::new();
        for o in &body.data.options {
            if o.iv <= 0.0 || o.open_interest <= 0.0 {
                continue;
            }
            if let Some((kind, expiry, strike)) = parse_occ(&o.option) {
                if crate::clock::alive(expiry, now) {
                    parsed.push((
                        expiry,
                        OptionContract {
                            kind,
                            strike,
                            expiry,
                            open_interest: o.open_interest,
                            volume: o.volume,
                            implied_vol: o.iv,
                        },
                    ));
                }
            }
        }
        let front = parsed
            .iter()
            .map(|(e, _)| *e)
            .min()
            .ok_or_else(|| anyhow!("cboe {ticker}: no usable contracts"))?;
        let contracts: Vec<OptionContract> = parsed
            .into_iter()
            .filter(|(e, _)| *e == front)
            .map(|(_, c)| c)
            .collect();

        // VIX context, best effort; the pipeline is fine without it.
        let vix = self.get("_VIX").await.ok().map(|v| v.data.current_price);

        Ok(ChainSnapshot {
            ticker: ticker.to_string(),
            spot,
            vix,
            taken_at: Utc::now(),
            contracts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_put() {
        let (kind, expiry, strike) = parse_occ("SPY260930P00696000").unwrap();
        assert_eq!(kind, OptionKind::Put);
        assert_eq!(expiry, NaiveDate::from_ymd_opt(2026, 9, 30).unwrap());
        assert_eq!(strike, 696.0);
    }

    #[test]
    fn parses_call_with_long_root_and_fractional_strike() {
        let (kind, expiry, strike) = parse_occ("SPXW261218C05002500").unwrap();
        assert_eq!(kind, OptionKind::Call);
        assert_eq!(expiry, NaiveDate::from_ymd_opt(2026, 12, 18).unwrap());
        assert_eq!(strike, 5002.5);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_occ("").is_none());
        assert!(parse_occ("SPY").is_none());
        assert!(parse_occ("SPY260930X00696000").is_none());
    }
}
