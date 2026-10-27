# syntax=docker/dockerfile:1.7

# ---- Build ----
FROM rust:1.99-slim-bookworm AS builder
WORKDIR /src

# Build dependencies first against stub sources so this layer is reused
# until Cargo.toml/Cargo.lock change.
COPY Cargo.toml Cargo.lock ./
COPY crates/domain/Cargo.toml crates/domain/
COPY crates/store/Cargo.toml crates/store/
COPY crates/http/Cargo.toml crates/http/
COPY crates/cli/Cargo.toml crates/cli/
RUN for crate in domain store http; do mkdir -p crates/$crate/src && echo "" > crates/$crate/src/lib.rs; done \
 && mkdir -p crates/cli/src && echo "fn main() {}" > crates/cli/src/main.rs \
 && cargo build --release --locked -p specsentry \
 && rm -rf crates/*/src

COPY crates crates
# Touch sources so cargo rebuilds the workspace crates, not the cached stubs.
RUN find crates -name '*.rs' -exec touch {} + \
 && cargo build --release --locked -p specsentry \
 && install -D target/release/specsentry /out/specsentry \
 && mkdir -p /out/data

# ---- Runtime ----
# distroless/cc: glibc + libgcc only, no shell or package manager; runs as uid 65532.
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
WORKDIR /app

COPY --from=builder /out/specsentry /usr/local/bin/specsentry
COPY --from=builder --chown=nonroot:nonroot /out/data /data
COPY public /app/public

ENV BIND_ADDR=0.0.0.0 \
    PORT=3005 \
    DATABASE_PATH=/data/specsentry.sqlite \
    PUBLIC_DIR=/app/public \
    LOG_FORMAT=json \
    RUST_LOG=info

USER nonroot:nonroot
VOLUME ["/data"]
EXPOSE 3005

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD ["/usr/local/bin/specsentry", "healthcheck"]

ENTRYPOINT ["/usr/local/bin/specsentry"]
CMD ["serve"]
