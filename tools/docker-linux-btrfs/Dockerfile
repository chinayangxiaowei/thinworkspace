FROM rust:1.97.1-bookworm@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97

RUN apt-get update \
    && apt-get install -y --no-install-recommends btrfs-progs musl-tools \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add clippy rustfmt \
    && rustup target add aarch64-unknown-linux-musl \
    && useradd --create-home --uid 1000 thinws

COPY --from=quality-tools quality-tools.toml /tmp/thinws-quality-tools.toml
RUN cargo_mutants_version="$(python3 -c 'import tomllib; print(tomllib.load(open("/tmp/thinws-quality-tools.toml", "rb"))["tools"]["cargo_mutants"])')" \
    && cargo install --locked --version "$cargo_mutants_version" cargo-mutants \
    && rm /tmp/thinws-quality-tools.toml
