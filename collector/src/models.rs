use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptionKind {
    Call,
    Put,
}

/// One option contract as delivered by a market-data provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionContract {
    pub kind: OptionKind,
    pub strike: f64,
    pub expiry: NaiveDate,
    pub open_interest: f64,
    pub volume: f64,
    pub implied_vol: f64, // annualised, e.g. 0.22
}

/// A full chain snapshot for one ticker at one moment.
#[derive(Debug, Clone)]
pub struct ChainSnapshot {
    pub ticker: String,
    pub spot: f64,
    pub vix: Option<f64>,
    pub taken_at: DateTime<Utc>,
    pub contracts: Vec<OptionContract>,
}

/// Per-strike exposure aggregates (the profile the dashboard draws).
#[derive(Debug, Clone, Serialize)]
pub struct StrikeExposure {
    pub strike: f64,
    pub call_gex: f64,
    pub put_gex: f64,
    pub net_gex: f64,
    pub net_dex: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Wall {
    pub strike: f64,
    pub gex: f64,
}

/// Everything the analytics layer derives from one chain snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct Analytics {
    pub ticker: String,
    pub taken_at: DateTime<Utc>,
    pub expiry: NaiveDate,
    pub spot: f64,
    pub net_gex_total: f64,
    pub abs_gex_total: f64,
    pub gamma_flip: Option<f64>,
    pub atm_iv: Option<f64>,
    pub net_delta_exposure: f64,
    pub expected_move_1sd: Option<f64>,
    pub charm_total: f64,
    pub vanna_total: f64,
    pub call_walls: Vec<Wall>,
    pub put_walls: Vec<Wall>,
    pub per_strike: Vec<StrikeExposure>,
    pub regime: String,
    pub signal_score: f64,
    pub traffic_light: String,
    pub vix: Option<f64>,
}
