FROM rust:1.97-slim AS build
WORKDIR /app
COPY schema.sql ./schema.sql
COPY collector ./collector
RUN cd collector && cargo build --release

FROM debian:bookworm-slim
COPY --from=build /app/collector/target/release/collector /usr/local/bin/collector
ENTRYPOINT ["collector"]
