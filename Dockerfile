FROM rust:1-slim-trixie AS builder

WORKDIR /app

RUN apt-get install -y --no-install-recommends ca-certificates
RUN update-ca-certificates

RUN --mount=type=bind,source=src,target=/app/src \
    --mount=type=bind,source=Cargo.toml,target=/app/Cargo.toml \
    --mount=type=bind,source=Cargo.lock,target=/app/Cargo.lock \
    --mount=type=cache,target=/app/target/ \
    --mount=type=cache,target=/usr/local/cargo/git/db \
    --mount=type=cache,target=/usr/local/cargo/registry/ \
    cargo build --locked --release && \
    cp ./target/release/roles-bot /bin/roles-bot

FROM debian:trixie-slim

COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=builder /bin/roles-bot /bin/

WORKDIR /data

CMD ["/bin/roles-bot"]
