FROM ubuntu:24.04
RUN apt-get update && apt-get install -y --no-install-recommends \
    gstreamer1.0-tools gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly ca-certificates \
    && rm -rf /var/lib/apt/lists/*
