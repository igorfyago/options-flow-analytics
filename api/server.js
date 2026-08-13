// API + dashboard for options-flow-analytics.
// Read-only over the snapshots the Rust collector writes.

const express = require("express");
const { Pool } = require("pg");
const path = require("path");

const pool = new Pool({
  connectionString:
    process.env.DATABASE_URL ||
    "postgres://postgres:postgres@localhost:5432/options",
});

const app = express();
// no-cache = store but revalidate: the dashboard is one html file, and a
// browser heuristically caching an old copy after a deploy is how other
// estate apps have shipped a crashed mixed page. ETag makes this a cheap 304.
app.use(express.static(path.join(__dirname, "public"), {
  setHeaders: (res) => res.setHeader("Cache-Control", "no-cache"),
}));

// Columns the dashboard needs (raw_options excluded to keep payloads lean).
const COLS = `id, timestamp, ticker, expiry, spot, regime, net_gex_total,
  abs_gex_total, gamma_flip, atm_iv, signal_score, traffic_light,
  recommendation, vix_current, expected_moves, call_walls, put_walls,
  charm_vanna, gex_per_strike, net_delta_exposure, market_regime`;

app.get("/api/tickers", async (_req, res) => {
  try {
    const { rows } = await pool.query(
      "SELECT DISTINCT ticker FROM gex_dex_snapshots ORDER BY ticker"
    );
    res.json(rows.map((r) => r.ticker));
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

app.get("/api/snapshots/:ticker/latest", async (req, res) => {
  try {
    const { rows } = await pool.query(
      `SELECT ${COLS} FROM gex_dex_snapshots
       WHERE ticker = $1 ORDER BY timestamp DESC LIMIT 1`,
      [req.params.ticker.toUpperCase()]
    );
    if (!rows.length) return res.status(404).json({ error: "no snapshots yet" });
    res.json(rows[0]);
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

app.get("/api/snapshots/:ticker/history", async (req, res) => {
  try {
    const limit = Math.min(parseInt(req.query.limit || "100", 10), 1000);
    const { rows } = await pool.query(
      `SELECT timestamp, spot, net_gex_total, gamma_flip, signal_score, traffic_light
       FROM gex_dex_snapshots
       WHERE ticker = $1 ORDER BY timestamp DESC LIMIT $2`,
      [req.params.ticker.toUpperCase(), limit]
    );
    res.json(rows.reverse());
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

app.get("/healthz", async (_req, res) => {
  try {
    await pool.query("SELECT 1");
    res.json({ ok: true });
  } catch (e) {
    res.status(500).json({ ok: false, error: e.message });
  }
});

const port = process.env.PORT || 3000;
app.listen(port, () => console.log(`api listening on :${port}`));
