# options-flow-analytics

Real-time options analytics: dealer positioning (GEX / DEX) computed live from option chains and flow, to read market regimes and generate trading signals. Dealer positioning is the most mechanically explainable force in intraday markets, so I built the system I wanted to exist.

> This repository is a working MVP of the architecture: Rust collector, PostgreSQL, Node.js API + dashboard, one command to run, no API keys needed (a synthetic market-data provider generates realistic chains). The production feed adapter I run privately stays private; the `MarketDataProvider` trait is where a real feed plugs in.

## Run it

```bash
docker compose up --build
# dashboard: http://localhost:3000
```

Local dev: `cd collector && cargo test && cargo run` (needs a local Postgres, see `DATABASE_URL` default) and `cd api && npm install && npm start`.

| Env | Default | Meaning |
|---|---|---|
| `TICKERS` | `SPY,QQQ` | comma-separated tickers to snapshot |
| `INTERVAL_SECS` | `30` | seconds between snapshot cycles |
| `PROVIDER` | `synthetic` | market-data source (`synthetic` ships with the MVP) |
| `SYNTHETIC_SPOT` | `500` | base spot for the synthetic walk |
| `RETENTION_DAYS` | `30` | pruning horizon for old snapshots |

## Architecture

```mermaid
flowchart LR
    API[External Market Data API] -->|REST + WebSocket| RUST

    subgraph RUST[Rust Collector - Tokio async]
        F[Fetch chains and flow] --> P[Parse and normalise]
        P --> C[Compute dealer positioning:<br/>GEX / DEX per strike, gamma flip,<br/>call/put walls, charm and vanna,<br/>expected moves, regime, signal score]
    end

    RUST -->|timestamped snapshots| PG[(PostgreSQL)]
    PG -->|SQL| NODE[Node.js / Express API]
    NODE -->|HTTP + JSON| UI[Web Dashboard]
```

Two microservices around a database, Dockerised, deployed on EC2 with PostgreSQL in a container.

## What it computes

- **Net / absolute GEX** per expiry and per strike, aggregated from open interest and greeks
- **Gamma flip level**: the spot price where dealer gamma changes sign, i.e. where hedging flows switch from dampening to amplifying moves
- **Call and put walls**: strike concentrations that act as magnets or barriers
- **Charm and vanna**: second-order greek exposures that drive pre-expiry drift
- **DEX profile**: net dealer delta exposure across the chain
- **Expected moves**, ATM IV, VIX context, VWAP, premarket context
- **Market-regime classification** and a composite signal score ("traffic light") with trade advice

## Data model

Snapshot-oriented: one row per (ticker, timestamp) with hot scalar columns for filtering and JSONB payloads for full per-strike profiles. Designed around the two query patterns that matter: "latest snapshot for ticker" and "everything for a day". See [`schema.sql`](schema.sql).

```sql
CREATE INDEX IF NOT EXISTS idx_snapshots_ticker_timestamp
    ON gex_dex_snapshots (ticker, timestamp DESC);
```

Retention is a pruning job, additive schema evolution only (new columns with defaults, JSONB extension points), so readers never break.

## Design decisions

- **Rust + Tokio** for the collector: sustained WebSocket ingestion and greek math on every snapshot, with predictable latency and no GC pauses
- **PostgreSQL over a time-series DB**: JSONB flexibility for evolving analytics beats specialised compression at this scale; the indexes carry the query patterns
- **Postgres in Docker on EC2 rather than RDS**: at personal scale, one instance with volume snapshots costs a fraction of managed Postgres, and I wanted to own the failure modes
- **Deterministic pipeline, no ML in the hot path**: positioning math is exact; interpretation is layered on top and can be rebuilt from raw snapshots (`raw_options` is kept per row)

## Layout

```
collector/   Rust: provider -> greeks -> analytics -> db   (unit-tested: cargo test)
api/         Node.js/Express API + self-contained dashboard (public/index.html)
schema.sql   the data model, embedded by the collector at compile time
```

## Stack

Rust (Tokio, tokio-postgres) · PostgreSQL · Node.js/Express · Docker Compose · AWS EC2
