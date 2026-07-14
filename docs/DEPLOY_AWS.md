# Deploying on AWS

Two tiers, matching how the system is actually meant to grow. The guiding
principle: at personal scale you pay for one box and own the failure modes;
you reach for managed services when a second consumer appears.

## Tier 1 — single EC2 (what I run)

```
Internet ──► Elastic IP ──► EC2 (t4g.small, ARM)
                              ├─ caddy          :443  TLS, reverse proxy → api
                              ├─ api            :3000 (Docker)
                              ├─ collector            (Docker)
                              └─ postgres       :5432 (Docker, volume on EBS gp3)
```

- **One `docker compose up -d` on cloud-init.** The compose file in this repo
  is the deployment artifact; nothing diverges between laptop and cloud.
- **PostgreSQL in a container, not RDS.** At one-writer scale, RDS buys
  backups, patching and failover you can replicate with an EBS snapshot
  policy at roughly a tenth of the cost — and operating Postgres yourself is
  exactly the muscle a personal system should train.
- **EBS gp3 + Data Lifecycle Manager**: nightly volume snapshots, 7-day
  retention. Restore = new volume from snapshot, reattach.
- **Security groups**: 443 from anywhere, 22 from my IP, nothing else.
  Postgres is never exposed; the API talks to it on the Docker network.
- **CloudWatch agent** ships container logs; one alarm on collector silence
  (no new snapshot row for 10 minutes) via a tiny cron + PutMetricData.
- Cost: ~€15–20/month.

## Tier 2 — when others start using it

| Concern | Move to |
|---|---|
| Database | RDS for PostgreSQL (Multi-AZ later), or Aurora Serverless v2 if usage is bursty |
| Containers | ECS on Fargate: `collector` as a service (desiredCount 1), `api` behind an ALB |
| Images | ECR, built and pushed from CI |
| Config/secrets | SSM Parameter Store; feed API keys in Secrets Manager |
| Cold data | Nightly `raw_options` offload from Postgres to S3 (Parquet), queryable via Athena |
| Observability | Container Insights + a CloudWatch dashboard mirroring the app's own /healthz |

The application code does not change between tiers: `DATABASE_URL` is the
only seam, which is the point of designing around one.

## Terraform

Both tiers are ~200 lines of Terraform (VPC, SG, instance + user-data or
ECS/ALB/RDS). Kept out of this repo to keep it focused; ask me for the walkthrough.
