# ---- Build Stage ----
FROM rust:latest as builder
WORKDIR /app
COPY . .
RUN cargo build --release

# ---- Runtime Stage ----
FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /app/target/release/payload-pulverizer /usr/local/bin/payload-pulverizer
COPY README.md ./
# Install SQLite3 (for rusqlite bundled)
RUN apt-get update && apt-get install -y libsqlite3-0 && rm -rf /var/lib/apt/lists/*
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/payload-pulverizer"] 