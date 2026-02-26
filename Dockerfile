FROM rust:1-slim as builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/agentmint /usr/local/bin/
EXPOSE 3000
CMD ["agentmint"]
