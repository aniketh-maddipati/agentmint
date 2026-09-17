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
EXPOSE 8787
CMD ["mint", "serve"]
