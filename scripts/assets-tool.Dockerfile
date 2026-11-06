# Tool image for the branding-asset check in `Lints (Fmt & Clippy)`.
#
# Issue #391: building this layer from `snapshot.debian.org` on every pull
# request meant a 503/504 from that CDN failed a required check before the asset
# comparison ever ran. The apt layer therefore tries the pinned snapshot first
# (reproducible package set) and falls back to `deb.debian.org` when the snapshot
# is unhealthy. The canonical asset output depends on the pinned ImageMagick
# base below, not on which mirror delivered python3/git.
#
# Publish it once with `.github/workflows/assets-tool-image.yml` and pin the
# digest in `scripts/assets-tool-image.txt`; CI then pulls instead of building.
FROM dpokidov/imagemagick:7.1.2-12@sha256:87998ec1b8127b2f73f626f74f7b05e8827f9d7605fa52da5370588f7e53cee1

RUN set -eux; \
    write_sources() { printf '%s\n' "$@" > /etc/apt/sources.list; rm -f /etc/apt/sources.list.d/*; }; \
    write_sources \
      "deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20250901T000000Z bullseye main" \
      "deb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/20250901T000000Z bullseye-security main"; \
    if ! { apt-get -o Acquire::Retries=5 -o Acquire::Check-Valid-Until=false update \
           && apt-get install -y --no-install-recommends python3 git; }; then \
      echo "assets tool image: snapshot.debian.org is unhealthy; falling back to deb.debian.org (the ImageMagick base stays pinned)" >&2; \
      write_sources \
        "deb http://deb.debian.org/debian bullseye main" \
        "deb http://deb.debian.org/debian-security bullseye-security main"; \
      apt-get -o Acquire::Retries=5 update; \
      apt-get install -y --no-install-recommends python3 git; \
    fi; \
    rm -rf /var/lib/apt/lists/*
