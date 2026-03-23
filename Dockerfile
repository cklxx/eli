# Build stage
FROM rust:1.85-bookworm AS builder

WORKDIR /app
COPY . .
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    tini && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/eli /usr/local/bin/eli

VOLUME /root/.eli

ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["eli"]
