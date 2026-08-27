FROM ubuntu:24.04@sha256:94f67e6e3f7f2d1e8f4f5d7b6f7f5b1f9a9d3b9c8d4b2f4f4e4b6d3f8a1e2c3
RUN apt-get update && apt-get install -y --no-install-recommends \
    gstreamer1.0-tools gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly ca-certificates \
    && rm -rf /var/lib/apt/lists/*
