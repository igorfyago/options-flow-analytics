//! Black-Scholes greeks (no dividends). Self-contained: normal pdf/cdf via
//! the Abramowitz & Stegun erf approximation, so no numeric crate is needed.

use crate::models::OptionKind;

const SQRT_2PI: f64 = 2.5066282746310002;

fn phi(x: f64) -> f64 {
    (-0.5 * x * x).exp() / SQRT_2PI
}

/// Abramowitz & Stegun 7.1.26, max abs error ~1.5e-7.
fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0
        - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t
            * (-x * x).exp();
    sign * y
}

fn norm_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

#[derive(Debug, Clone, Copy)]
pub struct Greeks {
    pub delta: f64,
    pub gamma: f64,
    /// dDelta/dVol (per 1.0 of vol, i.e. per 100 vol points)
    pub vanna: f64,
    /// dDelta/dTime (per year; negative drift of delta toward expiry)
    pub charm: f64,
}

/// `t_years` must be > 0; callers should skip expired contracts.
pub fn greeks(kind: OptionKind, spot: f64, strike: f64, t_years: f64, iv: f64, r: f64) -> Greeks {
    let sqrt_t = t_years.sqrt();
    let d1 = ((spot / strike).ln() + (r + 0.5 * iv * iv) * t_years) / (iv * sqrt_t);
    let d2 = d1 - iv * sqrt_t;
    let pdf_d1 = phi(d1);

    let delta = match kind {
        OptionKind::Call => norm_cdf(d1),
        OptionKind::Put => norm_cdf(d1) - 1.0,
    };
    let gamma = pdf_d1 / (spot * iv * sqrt_t);
    let vanna = -pdf_d1 * d2 / iv;
    // Same for calls and puts when q = 0.
    let charm = -pdf_d1 * (2.0 * r * t_years - d2 * iv * sqrt_t) / (2.0 * t_years * iv * sqrt_t);

    Greeks { delta, gamma, vanna, charm }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-3;

    #[test]
    fn atm_call_delta_near_half() {
        let g = greeks(OptionKind::Call, 100.0, 100.0, 30.0 / 365.0, 0.20, 0.05);
        assert!((g.delta - 0.5).abs() < 0.06, "ATM call delta ~0.5, got {}", g.delta);
    }

    #[test]
    fn put_call_delta_parity() {
        let c = greeks(OptionKind::Call, 100.0, 105.0, 0.25, 0.3, 0.02);
        let p = greeks(OptionKind::Put, 100.0, 105.0, 0.25, 0.3, 0.02);
        assert!((c.delta - p.delta - 1.0).abs() < EPS, "call - put delta must be 1");
        assert!((c.gamma - p.gamma).abs() < EPS, "gamma identical for calls and puts");
    }

    #[test]
    fn gamma_positive_and_peaks_atm() {
        let atm = greeks(OptionKind::Call, 100.0, 100.0, 0.1, 0.2, 0.02).gamma;
        let otm = greeks(OptionKind::Call, 100.0, 120.0, 0.1, 0.2, 0.02).gamma;
        assert!(atm > 0.0 && otm > 0.0);
        assert!(atm > otm, "gamma should peak near the money");
    }

    #[test]
    fn cdf_sanity() {
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-7);
        assert!(norm_cdf(3.0) > 0.998);
        assert!(norm_cdf(-3.0) < 0.002);
    }
}
