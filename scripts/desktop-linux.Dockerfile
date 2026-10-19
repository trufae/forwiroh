FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential curl ca-certificates pkg-config libgtk-4-dev libjson-glib-dev \
    valac meson ninja-build xvfb xauth python3 dbus-x11 && rm -rf /var/lib/apt/lists/*
RUN useradd --create-home --uid 1001 forwiroh
USER forwiroh
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH="/home/forwiroh/.cargo/bin:${PATH}"
WORKDIR /src
