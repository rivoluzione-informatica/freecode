# FreeCode `run` sandbox (RFC-002 Slice 3) — the isolation boundary for the gated `run` tool.
#
# Build once:
#   docker build -t freecode-sandbox -f docker/freecode-sandbox.Dockerfile .
#
# At run time FreeCode invokes it ephemerally and WITHOUT network:
#   docker run --rm --network none -v <workspace>:<workspace> -w <workspace> freecode-sandbox <cmd>
# so this image only needs the toolchains — it never has network access while a command runs.
# The deterministic command policy still applies (Deny/Approve/Allow); the container is defense
# in depth (no egress, ephemeral FS outside the mounted workspace, process isolation).
#
# Only Allow-class commands ever reach here, and AUTO mode runs ONLY through this container.

# Digest-pinned: a bare `debian:bookworm-slim` tag is mutable, so two builds of the same
# Dockerfile could produce different images. Refresh deliberately, not by accident.
FROM debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818

# Toolchains the Allow-list commands need: build/test runners + search/git. Installed at BUILD time
# (network available); runtime is --network none.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl git build-essential pkg-config \
        ripgrep python3 python3-pip python3-venv nodejs npm \
    && rm -rf /var/lib/apt/lists/*

# Rust toolchain. The old form was `curl https://sh.rustup.rs | sh` — an unverified
# pipe-to-shell: whatever the endpoint serves at build time executes as root, with no way to
# tell afterwards what ran. Download, checksum against a pinned rustup version, THEN execute.
ARG RUSTUP_VERSION=1.28.2
ARG RUST_VERSION=1.90.0
RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
      amd64) target='x86_64-unknown-linux-gnu'; sha='20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c' ;; \
      arm64) target='aarch64-unknown-linux-gnu'; sha='e3853c5a252fca15252d07cb23a1bdd9377a8c6f3efa01531109281ae47f841c' ;; \
      *) echo "unsupported architecture: $arch" >&2; exit 1 ;; \
    esac; \
    url="https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${target}/rustup-init"; \
    curl -sSfL "$url" -o /tmp/rustup-init; \
    echo "${sha}  /tmp/rustup-init" | sha256sum -c -; \
    chmod +x /tmp/rustup-init; \
    /tmp/rustup-init -y --no-modify-path --profile minimal \
        --default-toolchain "${RUST_VERSION}" --component clippy rustfmt; \
    rm -f /tmp/rustup-init

ENV PATH="/root/.cargo/bin:${PATH}"
ENV CARGO_TERM_COLOR=never

# `curl` is only needed at build time. Leaving it in the image hands any command that does
# escape the policy a ready-made exfil tool — remove it from the runtime image.
# (`--network none` already blocks egress; this is the second lock on the same door.)
RUN apt-get purge -y curl && apt-get autoremove -y && rm -rf /var/lib/apt/lists/*

# The workspace is bind-mounted and `-w`-set at run time, so no fixed WORKDIR is required.
#
# NOTE ON UID: this image deliberately stays root. The workspace is bind-mounted from the host
# and commands must write build artifacts back into it (target/, node_modules/), which a fixed
# non-root UID cannot do without matching the host owner. The isolation that carries the weight
# here is `--network none` + `--rm` + the deterministic Allow-list, not the in-container UID.
# If you run this on shared infrastructure, invoke it with `--user "$(id -u):$(id -g)"`.
CMD ["bash"]
