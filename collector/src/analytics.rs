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
    // Every qualifying contract's (distance from spot, iv). The ATM vol is a
    // MEDIAN over the strikes nearest spot rather than one contract's quote,
    // because a single bad or one-sided print used to set the whole surface.
    let mut atm_pool: Vec<(f64, f64)> = Vec::new();

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

        atm_pool.push(((c.strike - spot).abs(), c.implied_vol));
    }

    let per_strike: Vec<StrikeExposure> = strikes.into_values().collect();
    let net_gex_total: f64 = per_strike.iter().map(|s| s.net_gex).sum();
    let abs_gex_total: f64 = per_strike.iter().map(|s| s.net_gex.abs()).sum();

    let gamma_flip = find_flip(&per_strike, spot);
    let atm_iv = atm_iv_median(&mut atm_pool, spot);

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

/// How far from spot a contract may sit and still count as at the money.
const ATM_IV_BAND: f64 = 0.02;

/// ATM implied vol: the MEDIAN iv of the contracts closest to spot.
///
/// This used to be whichever single contract sat nearest spot and arrived first,
/// so one stale or one-sided quote could set the entire vol surface. Taking a
/// median over both sides of the ATM strike and its neighbours survives a bad
/// print. This is hardening, not a bug fix: it was checked against a live CBOE
/// chain where SPY's near-ATM contracts all agreed at about 10.4%, and the
/// median returned the same number the single-contract version did.
///
/// A wide SPY-versus-QQQ vol gap is NOT evidence of a fault here. On the 2026-07-20
/// expiry SPY printed about 10.4% and QQQ about 19.4% from the same feed on the
/// same tick, both tightly clustered across calls and puts with real open
/// interest. That is term structure and single-name event risk, not bad data.
fn atm_iv_median(pool: &mut Vec<(f64, f64)>, spot: f64) -> Option<f64> {
    if pool.is_empty() {
        return None;
    }
    pool.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Bound by DISTANCE first, not just by count: a thin chain has fewer than
    // six contracts near the money, and taking six regardless reaches into the
    // wings, where skew drags the median far above the real ATM vol.
    let band = ATM_IV_BAND * spot;
    let near = pool.iter().take_while(|(d, _)| *d <= band).count();
    let take = near.clamp(1, 6);
    let mut ivs: Vec<f64> = pool[..take].iter().map(|(_, iv)| *iv).collect();
    ivs.sort_by(|a, b| a.total_cmp(b));
    let mid = ivs.len() / 2;
    Some(if ivs.len() % 2 == 0 {
        (ivs[mid - 1] + ivs[mid]) / 2.0
    } else {
        ivs[mid]
    })
}

/// Fraction of the chain's total absolute gamma that a zero-crossing must
/// actually move for it to count as the flip.
///
/// Deep-OTM strikes carry gamma that is not zero but is numerical dust: on a
/// real SPY chain the 21 strikes at or below 640 peaked at 0.23 of net_gex
/// while the chain totalled 1.16e9. The cumulative sum wanders across zero
/// down there for no economic reason, and a scan that takes the FIRST such
/// crossing reported a flip at 620 with spot at 746 - a "tipping point" set
/// by 0.0014 of gamma, roughly 1e-10 of the book. Requiring a crossing to
/// swing a real share of the chain discards that dust.
const FLIP_MIN_SHARE: f64 = 0.01;

/// Gamma flip: the spot level where cumulative net GEX crosses zero,
/// linearly interpolated between strikes.
///
/// Two rules keep the answer honest, and both exist because their absence
/// produced a wrong number in production:
///
/// 1. A crossing only counts if the cumulative gamma on one side of it is at
///    least `FLIP_MIN_SHARE` of the chain. Otherwise it is float dust.
/// 2. Of the crossings that qualify, the one NEAREST SPOT wins. Scanning from
///    the lowest strike and returning the first hit picks whichever crossing
///    happens to sit furthest from the money, which is the least relevant one
///    to today's tape.
///
/// Returning None is a legitimate answer: a chain whose cumulative gamma never
/// meaningfully changes sign has no tipping point to trade against, and saying
/// so beats inventing a level.
fn find_flip(per_strike: &[StrikeExposure], spot: f64) -> Option<f64> {
    let total: f64 = per_strike.iter().map(|s| s.net_gex.abs()).sum();
    if total <= 0.0 {
        return None;
    }
    let floor = total * FLIP_MIN_SHARE;

    let mut cum = 0.0;
    let mut prev: Option<(f64, f64)> = None; // (strike, cumulative through it)
    let mut best: Option<f64> = None;
    for s in per_strike {
        let next = cum + s.net_gex;
        if let Some((p_strike, p_cum)) = prev {
            let crosses = (p_cum < 0.0 && next >= 0.0) || (p_cum > 0.0 && next <= 0.0);
            // the swing has to be real on at least one side, or it is dust
            if crosses && p_cum.abs().max(next.abs()) >= floor {
                let span = next - p_cum;
                let level = if span.abs() > f64::EPSILON {
                    p_strike + ((0.0 - p_cum) / span) * (s.strike - p_strike)
                } else {
                    s.strike
                };
                if best.is_none_or(|b| (level - spot).abs() < (b - spot).abs()) {
                    best = Some(level);
                }
            }
        }
        prev = Some((s.strike, next));
        cum = next;
    }
    best
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

    /// Build a per-strike book directly, so the dust can be stated exactly.
    fn exposures(rows: &[(f64, f64)]) -> Vec<StrikeExposure> {
        rows.iter()
            .map(|&(strike, net_gex)| StrikeExposure {
                strike,
                net_gex,
                net_dex: 0.0,
                put_gex: net_gex.min(0.0),
                call_gex: net_gex.max(0.0),
            })
            .collect()
    }

    #[test]
    fn float_dust_far_from_spot_is_not_a_flip() {
        // The production shape, 2026-07-20: spot 746.75, deep-OTM strikes
        // carrying ~1e-5 of gamma that wobbles across zero, and the real book
        // (hundreds of millions) sitting near the money and staying negative.
        // The old scan returned 620 - a flip 127 points away decided by a
        // rounding error. There is no true flip here, so the answer is None.
        let mut rows = vec![
            (500.0, -1.76e-05),
            (620.0, 3.10e-05),   // the dust crossing the old code returned
            (625.0, -2.00e-05),
        ];
        for k in 740..=755 {
            rows.push((k as f64, -50_000_000.0)); // real gamma, one-sided
        }
        assert_eq!(find_flip(&exposures(&rows), 746.75), None);
    }

    #[test]
    fn flip_nearest_spot_wins_over_a_distant_one() {
        // Two crossings that both clear the noise floor. The one by the money
        // is the tradable tipping point; the far one is history.
        let rows = vec![
            (600.0, -1_000_000.0),
            (610.0, 2_000_000.0),   // crossing far below spot
            (740.0, -3_000_000.0),  // crossing near spot
            (750.0, 4_000_000.0),
        ];
        let flip = find_flip(&exposures(&rows), 746.0).expect("flip expected");
        assert!(flip > 700.0, "picked the distant crossing: {flip}");
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

#[cfg(test)]
mod atm_iv_tests {
    use super::*;

    #[test]
    fn one_bad_quote_at_the_money_cannot_set_the_surface() {
        // The SPY case: the contract nearest spot printed a nonsense 10.5%
        // while everything around it agreed on roughly 19%. First-wins picked
        // the outlier; the median must ignore it.
        let mut pool = vec![
            (0.0, 0.105),
            (0.5, 0.191),
            (0.5, 0.193),
            (1.0, 0.190),
            (1.0, 0.195),
            (1.5, 0.192),
            (40.0, 0.60),   // far wing, must not be reached
        ];
        let iv = atm_iv_median(&mut pool, 100.0).unwrap();
        assert!(iv > 0.18 && iv < 0.20, "atm iv {iv} should sit with the cluster");
    }

    #[test]
    fn far_wings_are_excluded_even_when_numerous() {
        let mut pool = vec![(0.0, 0.20), (0.5, 0.20), (1.0, 0.20)];
        for i in 0..50 {
            pool.push((100.0 + i as f64, 0.90));   // deep OTM skew
        }
        let iv = atm_iv_median(&mut pool, 100.0).unwrap();
        assert!((iv - 0.20).abs() < 1e-9, "wings leaked into atm iv: {iv}");
    }

    #[test]
    fn an_empty_chain_has_no_atm_iv() {
        assert!(atm_iv_median(&mut Vec::new(), 100.0).is_none());
    }
}
