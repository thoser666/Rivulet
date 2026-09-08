#!/usr/bin/env python3
"""Single source of truth for the wiki locales used by the sync scripts.

Kept importable without pulling in argparse or repo paths so both
`sync-wiki-translations.py` and `check-wiki-translations.py` can share the
same list. English is canonical (no suffix); every other entry is a valid
`-<locale>` page suffix.
"""
from __future__ import annotations

# Canonical language has no suffix; the rest are page suffixes.
KNOWN_LOCALES = ("de", "es", "fr")