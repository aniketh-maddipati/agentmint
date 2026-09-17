FROM rust:1-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin mint

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/mint /usr/local/bin/mint
ENV MINT_BIND_ADDR=0.0.0.0:8787
ENV MINT_MODE=development
ENV MINT_PROVIDER=fake
ENV MINT_IDENTITY=local
ENV MINT_DATABASE_PATH=/data/mint.db
ENV MINT_SIGNING_KEY_FILE=/keys/mint.ed25519.pem
VOLUME ["/data", "/keys"]
EXPOSE 8787
# Provide a key first, e.g.:
#   docker run --rm -v "$PWD/keys:/keys" mint-run mint init --key-file /keys/mint.ed25519.pem
#   docker run --rm -p 8787:8787 -v "$PWD/keys:/keys" -v "$PWD/data:/data" mint-run mint serve
CMD ["mint", "serve"]
