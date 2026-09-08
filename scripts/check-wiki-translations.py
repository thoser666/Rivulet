#!/usr/bin/env python3
"""Validate bilingual GitHub Wiki page pairs.

The canonical language is English (pages without a locale suffix); every
configured additional locale must exist as a `<page>-<locale>.md` pair with
language-switch links in both directions. The locale list is configurable via
`--locales` (comma-separated, default `de`) so adding a new language later
(e.g. `--locales de,es,fr`) extends checking automatically without changing
this script.

Exit codes: 0 = all pairs valid; 1 = missing pairs/links; 2 = usage error.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import sys

from check_wiki_locales import KNOWN_LOCALES

# Language-switch link on a canonical page: "[Deutsch](Page-de)" etc.
SWITCH_LINK_RE = re.compile(
    r"\[([^\]]+)\]\(([^)#?\s]+)\)", re.IGNORECASE
)

# Marker words that identify a language-switch link for a locale. English is
# always the canonical language; other locales are keyed by suffix.
LOCALE_LINK_WORDS = {
    "de": ("deutsch", "german"),
    "en": ("english", "englisch"),
    "es": ("spanish", "spanisch", "español", "espanol"),
    "fr": ("french", "französisch", "franzosisch", "français", "francais"),
}


def parse_locales(value: str) -> list[str]:
    """Parse the --locales value; raises ValueError on invalid input."""
    locales = []
    for part in value.split(","):
        part = part.strip().lower()
        if part and part not in locales:
            locales.append(part)
    if not locales:
        raise ValueError("--locales must not be empty")
    if "en" in locales:
        # English is canonical; a duplicate suffix would be meaningless.
        locales = [loc for loc in locales if loc != "en"]
    if len(set(locales)) != len(locales):
        raise ValueError("duplicate locale in --locales")
    for loc in locales:
        if loc not in LOCALE_LINK_WORDS:
            raise ValueError(
                f"unknown locale '{loc}' (known: {', '.join(sorted(LOCALE_LINK_WORDS))})"
            )
    return locales


def link_words_for(locale: str) -> tuple[str, ...]:
    return LOCALE_LINK_WORDS[locale]


def has_switch_link(text: str, locale: str) -> bool:
    words = link_words_for(locale)
    return any(
        any(word in label.lower() for word in words)
        for label, _ in SWITCH_LINK_RE.findall(text)
    )


def check(root: Path, locales: list[str]) -> list[str]:
    errors: list[str] = []
    english = sorted(p for p in root.glob("*.md") if not _has_locale_suffix(p.stem, locales))
    for source in english:
        if source.name == "Languages.md":
            continue
        text = source.read_text(encoding="utf-8")
        for locale in locales:
            translated = root / f"{source.stem}-{locale}.md"
            if not translated.exists():
                errors.append(f"missing {locale} pair: {source.name} -> {translated.name}")
                continue
            translated_text = translated.read_text(encoding="utf-8")
            if not has_switch_link(text, locale):
                errors.append(f"missing {locale} language link: {source.name}")
            if not has_switch_link(translated_text, "en"):
                errors.append(f"missing English language link: {translated.name}")
    return errors


def _has_locale_suffix(stem: str, locales: list[str]) -> bool:
    # Any known-locale suffix marks a translated page, even when that locale
    # is not currently configured — a Spanish page must never be mistaken for
    # a canonical English page just because only German checking is enabled.
    return any(stem.endswith(f"-{locale}") for locale in KNOWN_LOCALES)


def self_test() -> int:
    """Offline logic test with a temp directory (no network, no wiki clone)."""
    import tempfile

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "Home.md").write_text(
            "# Home\n\n[English](Home) · [Deutsch](Home-de) · [Español](Home-es)\n",
            encoding="utf-8",
        )
        (root / "Home-de.md").write_text(
            "[English](Home) · [Deutsch](Home-de)\n\n# Startseite\n", encoding="utf-8"
        )
        (root / "Home-es.md").write_text(
            "[English](Home) · [Español](Home-es)\n\n# Inicio\n", encoding="utf-8"
        )
        (root / "Orphan.md").write_text("# Orphan\n\nno switch links\n", encoding="utf-8")

        # All three locales configured: Home is fine, Orphan misses every pair.
        errors = check(root, ["de", "es"])
        assert any("missing de pair: Orphan.md" in e for e in errors), errors
        assert any("missing es pair: Orphan.md" in e for e in errors), errors
        assert not any("Home" in e for e in errors), errors

        # Only German configured: the Spanish pair is irrelevant, still no Home errors.
        errors = check(root, ["de"])
        assert any("missing de pair: Orphan.md" in e for e in errors), errors
        assert not any("Home" in e for e in errors), errors

        # Break the German switch link on Home (only the English link remains):
        # must be flagged for de only. Also drop the English link on the German
        # page so the reverse direction is flagged too.
        (root / "Home.md").write_text("# Home\n\n[English](Home)\n", encoding="utf-8")
        (root / "Home-de.md").write_text("[Deutsch](Home-de)\n\n# Startseite\n", encoding="utf-8")
        errors = check(root, ["de"])
        assert any("missing de language link: Home.md" in e for e in errors), errors
        assert any("missing English language link: Home-de.md" in e for e in errors), errors

        # Locale parsing: 'en' is filtered (canonical), whitespace tolerated,
        # duplicates collapse, unknown locales rejected.
        assert parse_locales("de,en") == ["de"]
        assert parse_locales("de, de ,es") == ["de", "es"]
        assert parse_locales("de,de") == ["de"]
        for bad in ("", "xx"):
            try:
                parse_locales(bad)
            except ValueError:
                pass
            else:
                raise AssertionError(f"parse_locales({bad!r}) must fail")

        # Suffix detection drives which pages are canonical: with locale es
        # configured, Home-es.md is NOT checked as a canonical page itself.
        english = sorted(p.name for p in root.glob("*.md") if not _has_locale_suffix(p.stem, ["de", "es"]))
        assert "Home-es.md" not in english and "Home-de.md" not in english, english

    print("self-test OK")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", default=".freebuff-rivulet-wiki")
    parser.add_argument(
        "--locales",
        default="de",
        help="comma-separated additional locales (default: de; en is canonical)",
    )
    parser.add_argument("--self-test", action="store_true", help="run the offline logic test")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    try:
        locales = parse_locales(args.locales)
    except ValueError as exc:
        print(f"check-wiki-translations: {exc}")
        return 2
    errors = check(Path(args.root), locales)
    if errors:
        print("wiki translation check failed:")
        print("\n".join(f"- {error}" for error in errors))
        return 1
    print(f"wiki translation check passed (locales: {', '.join(['en'] + locales)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
