# syntax=docker/dockerfile:1

# ============================================================
# MengDrive — Telegram S3 Object Storage Engine
#
# The binary is BUILT LOCALLY (make build / pre-push hook) and
# only the finished artifact is copied into this image.
# No Rust toolchain, no compilation, no source code inside.
# ============================================================

FROM debian:bookworm-slim

ARG APP_USER=mengdrive
ARG APP_UID=10001

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
    && rm -rf /var/lib/apt/lists/*

# Non-root user
RUN useradd --system --uid "${APP_UID}" --user-group "${APP_USER}"

WORKDIR /app

# Only the prebuilt artifact enters the image.
# Build it first: make build  (release binary at target/release/s3-telegram)
COPY target/release/s3-telegram /usr/local/bin/s3-telegram

# Tera templates are loaded at runtime (templates/**/*)
COPY templates /app/templates

RUN chown -R "${APP_USER}:${APP_USER}" /app

USER ${APP_USER}

ENV SERVER_PORT=3060
EXPOSE 3060

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://127.0.0.1:${SERVER_PORT}/ || exit 1

ENTRYPOINT ["/usr/local/bin/s3-telegram"]
