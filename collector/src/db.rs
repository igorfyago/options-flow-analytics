//! Persistence: schema init, snapshot insert, retention pruning.
//! The schema lives in ../schema.sql at the repo root and is embedded at
//! compile time so the collector is self-bootstrapping on a fresh database.

use crate::models::Analytics;
use anyhow::Result;
use tokio_postgres::{Client, NoTls};

const SCHEMA: &str = include_str!("../../schema.sql");

pub async fn connect(url: &str) -> Result<Client> {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("db connection error: {e}");
        }
    });
    client.batch_execute(SCHEMA).await?;
    Ok(client)
}

pub async fn insert_snapshot(client: &Client, a: &Analytics) -> Result<()> {
    let sql = r#"
        INSERT INTO gex_dex_snapshots (
            timestamp, ticker, expiry, spot, regime,
            net_gex_total, abs_gex_total, gamma_flip, atm_iv,
            signal_score, traffic_light, recommendation, vix_current,
            expected_moves, call_walls, put_walls, charm_vanna,
            gex_per_strike, delta_exposure_profile, net_delta_exposure,
            raw_options, market_regime
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22)
    "#;

    let recommendation = serde_json::json!({
        "signal": a.traffic_light,
        "score": a.signal_score,
        "note": "MVP composite: gamma regime + spot-vs-flip distance",
    });
    let expected_moves = serde_json::json!([{
        "horizon": "expiry",
        "one_sigma": a.expected_move_1sd,
    }]);
    let charm_vanna = serde_json::json!({
        "charm_total": a.charm_total,
        "vanna_total": a.vanna_total,
    });
    let per_strike = serde_json::to_value(&a.per_strike)?;
    let dex_profile = serde_json::json!(a
        .per_strike
        .iter()
        .map(|s| serde_json::json!({"strike": s.strike, "net_dex": s.net_dex}))
        .collect::<Vec<_>>());
    let call_walls = serde_json::to_value(&a.call_walls)?;
    let put_walls = serde_json::to_value(&a.put_walls)?;

    client
        .execute(
            sql,
            &[
                &a.taken_at,
                &a.ticker,
                &a.expiry,
                &a.spot,
                &a.regime,
                &a.net_gex_total,
                &a.abs_gex_total,
                &a.gamma_flip,
                &a.atm_iv,
                &a.signal_score,
                &a.traffic_light,
                &recommendation,
                &a.vix,
                &expected_moves,
                &call_walls,
                &put_walls,
                &charm_vanna,
                &per_strike,
                &dex_profile,
                &a.net_delta_exposure,
                &serde_json::json!([]), // raw_options: omitted in MVP to keep rows lean
                &a.regime,
            ],
        )
        .await?;
    Ok(())
}

pub async fn prune(client: &Client, retention_days: f64) -> Result<u64> {
    let n = client
        .execute(
            "DELETE FROM gex_dex_snapshots WHERE timestamp < NOW() - INTERVAL '1 day' * $1",
            &[&retention_days],
        )
        .await?;
    Ok(n)
}
