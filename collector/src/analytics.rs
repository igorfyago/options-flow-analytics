//! Dealer-positioning analytics: GEX/DEX per strike, gamma flip, walls,
//! charm/vanna aggregates, expected move, regime and a composite signal.
//!
//! Conventions (the common retail-GEX model):
//!   - dealers are assumed long calls (+gamma) and short puts (-gamma)
//!   - GEX per strike = gamma * OI * 100 * spot^2 * 0.01  (dollar gamma per 1% move)
//!   - DEX per strike = delta * OI * 100 * spot

use crate::greeks::greeks;
use crate::models::*;
use chrono::Utc;
use std::collections::BTreeMap;

const RISK_FREE: f64 = 0.04;
const CONTRACT_MULT: f64 = 100.0;

pub fn compute(chain: &ChainSnapshot) -> Analytics {
    let spot = chain.spot;
    let today = Utc::now().date_naive();

    // Aggregate per strike. BTreeMap keeps strikes ordered for the flip scan.
    let mut strikes: BTreeMap<i64, StrikeExposure> = BTreeMap::new();
    let (mut net_dex, mut charm_total, mut vanna_total) = (0.0, 0.0, 0.0);
    let mut expiry = None;
    let mut atm: Option<(f64, f64)> = None; // (distance, iv)

    for c in &chain.contracts {
        let dte = (c.expiry - today).num_days().max(0) as f64;
        if dte == 0.0 || c.implied_vol <= 0.0 || c.open_interest <= 0.0 {
            continue;
        }
        expiry.get_or_insert(c.expiry);
        let t = dte / 365.0;
        let g = greeks(c.kind, spot, c.strike, t, c.implied_vol, RISK_FREE);

        let oi = c.open_interest;
        let gex = g.gamma * oi * CONTRACT_MULT * spot * spot * 0.01;
        let signed_gex = match c.kind {
            OptionKind::Call => gex,
            OptionKind::Put => -gex,
        };
        let dex = g.delta * oi * CONTRACT_MULT * spot;

        let e = strikes.entry(c.strike as i64).or_insert(StrikeExposure {
            strike: c.strike,
            call_gex: 0.0,
            put_gex: 0.0,
            net_gex: 0.0,
            net_dex: 0.0,
        });
        match c.kind {
            OptionKind::Call => e.call_gex += gex,
            OptionKind::Put => e.put_gex -= gex,
        }
        e.net_gex += signed_gex;
        e.net_dex += dex;
        net_dex += dex;
        charm_total += g.charm * oi * CONTRACT_MULT;
        vanna_total += g.vanna * oi * CONTRACT_MULT;

        let dist = (c.strike - spot).abs();
        if atm.map_or(true, |(d, _)| dist < d) {
            atm = Some((dist, c.implied_vol));
        }
    }

    let per_strike: Vec<StrikeExposure> = strikes.into_values().collect();
    let net_gex_total: f64 = per_strike.iter().map(|s| s.net_gex).sum();
    let abs_gex_total: f64 = per_strike.iter().map(|s| s.net_gex.abs()).sum();

    let gamma_flip = find_flip(&per_strike);
    let atm_iv = atm.map(|(_, iv)| iv);

    // 1-sigma expected move to the chain's expiry.
    let expected_move_1sd = match (atm_iv, expiry) {
        (Some(iv), Some(exp)) => {
            let dte = (exp - today).num_days().max(0) as f64;
            Some(spot * iv * (dte / 365.0).sqrt())
        }
        _ => None,
    };

    // Walls: biggest positive-GEX strike above spot (call wall) and biggest
    // negative below (put wall), top 3 each.
    let mut calls: Vec<Wall> = per_strike
        .iter()
        .filter(|s| s.strike >= spot && s.net_gex > 0.0)
        .map(|s| Wall { strike: s.strike, gex: s.net_gex })
        .collect();
    calls.sort_by(|a, b| b.gex.total_cmp(&a.gex));
    calls.truncate(3);

    let mut puts: Vec<Wall> = per_strike
        .iter()
        .filter(|s| s.strike <= spot && s.net_gex < 0.0)
        .map(|s| Wall { strike: s.strike, gex: s.net_gex })
        .collect();
    puts.sort_by(|a, b| a.gex.total_cmp(&b.gex));
    puts.truncate(3);

    let regime = if net_gex_total >= 0.0 { "positive_gamma" } else { "negative_gamma" };

    // Composite signal in [-1, 1]: gamma regime + where spot sits vs the flip.
    let mut score = if net_gex_total >= 0.0 { 0.4 } else { -0.4 };
    if let Some(flip) = gamma_flip {
        score += ((spot - flip) / spot * 20.0).clamp(-0.6, 0.6);
    }
    let score = score.clamp(-1.0, 1.0);
    let traffic_light = if score > 0.25 {
        "green"
    } else if score < -0.25 {
        "red"
    } else {
        "amber"
    };

    Analytics {
        ticker: chain.ticker.clone(),
        taken_at: chain.taken_at,
        expiry: expiry.unwrap_or(today),
        spot,
        net_gex_total,
        abs_gex_total,
        gamma_flip,
        atm_iv,
        net_delta_exposure: net_dex,
        expected_move_1sd,
        charm_total,
        vanna_total,
        call_walls: calls,
        put_walls: puts,
        per_strike,
        regime: regime.to_string(),
        signal_score: score,
        traffic_light: traffic_light.to_string(),
        vix: chain.vix,
    }
}

/// Gamma flip: the strike level where cumulative net GEX (scanned from the
/// lowest strike upward) crosses zero, linearly interpolated between strikes.
fn find_flip(per_strike: &[StrikeExposure]) -> Option<f64> {
    let mut cum = 0.0;
    let mut prev: Option<(f64, f64)> = None; // (strike, cum before this strike)
    for s in per_strike {
        let next = cum + s.net_gex;
        if let Some((p_strike, p_cum)) = prev {
            if (p_cum < 0.0 && next >= 0.0) || (p_cum > 0.0 && next <= 0.0) {
                let span = next - p_cum;
                if span.abs() > f64::EPSILON {
                    let frac = (0.0 - p_cum) / span;
                    return Some(p_strike + frac * (s.strike - p_strike));
                }
                return Some(s.strike);
            }
        }
        prev = Some((s.strike, next));
        cum = next;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn chain(contracts: Vec<OptionContract>) -> ChainSnapshot {
        ChainSnapshot {
            ticker: "TEST".into(),
            spot: 100.0,
            vix: None,
            taken_at: Utc::now(),
            contracts,
        }
    }

    fn contract(kind: OptionKind, strike: f64, oi: f64) -> OptionContract {
        OptionContract {
            kind,
            strike,
            expiry: (Utc::now() + Duration::days(14)).date_naive(),
            open_interest: oi,
            volume: 0.0,
            implied_vol: 0.20,
        }
    }

    #[test]
    fn calls_positive_puts_negative() {
        let a = compute(&chain(vec![contract(OptionKind::Call, 100.0, 1000.0)]));
        assert!(a.net_gex_total > 0.0);
        let a = compute(&chain(vec![contract(OptionKind::Put, 100.0, 1000.0)]));
        assert!(a.net_gex_total < 0.0);
    }

    #[test]
    fn flip_sits_between_put_and_call_mass() {
        // Heavy puts low, heavy calls high: cumulative GEX goes negative then
        // positive, so a flip must exist between 90 and 110.
        let a = compute(&chain(vec![
            contract(OptionKind::Put, 90.0, 5000.0),
            contract(OptionKind::Call, 110.0, 5000.0),
        ]));
        let flip = a.gamma_flip.expect("flip expected");
        assert!(flip > 90.0 && flip <= 110.0, "flip {flip} out of range");
    }

    #[test]
    fn regime_follows_net_gex_sign() {
        let a = compute(&chain(vec![contract(OptionKind::Call, 100.0, 1000.0)]));
        assert_eq!(a.regime, "positive_gamma");
        let a = compute(&chain(vec![contract(OptionKind::Put, 100.0, 1000.0)]));
        assert_eq!(a.regime, "negative_gamma");
    }

    #[test]
    fn expired_contracts_are_ignored() {
        let mut c = contract(OptionKind::Call, 100.0, 1000.0);
        c.expiry = Utc::now().date_naive(); // 0 DTE -> skipped
        let a = compute(&chain(vec![c]));
        assert_eq!(a.net_gex_total, 0.0);
        assert!(a.per_strike.is_empty());
    }
}
