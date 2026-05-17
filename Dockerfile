FROM rust:1-bookworm AS builder

WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libopus0 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/cname-blocker-voip /usr/local/bin/cname-blocker-voip

ENTRYPOINT ["/usr/local/bin/cname-blocker-voip"]
