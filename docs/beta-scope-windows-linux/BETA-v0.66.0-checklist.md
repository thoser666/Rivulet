# Erstes Beta: v0.66.0 — Was noch nötig ist

## Voraussetzungen (Beta-Gate)

Der echte Gate-Check steht in `scripts/check-beta-gate.py` und wird mit Token ausgewertet:

```
GH_TOKEN="$(gh auth token)" python3 scripts/check-beta-gate.py
```

**Aktuell:** `NOT READY (1 not met)`. Fehlendes Kriterium:

| # | Kriterium | Status | Was fehlt |
|---|---|---|---|
| 4 | Code-Signing-Secrets konfiguriert | **not-met** | 11 Secrets nicht vorhanden |
| 1,2,3,5,6 | M1/M3, Platform-Parity, CI-green, keine release-blockers | ✅ met | — |

**Fehlende Secrets im Repository (current state via PAT):**

```
[
  'WINDOWS_CERT_BASE64', 'WINDOWS_CERT_PASSWORD',
  'SIGNPATH_API_TOKEN', 'SIGNPATH_ORGANIZATION_ID', 'SIGNPATH_PROJECT_SLUG', 'SIGNPATH_SIGNING_POLICY_SLUG',
  'MACOS_CERT_BASE64', 'MACOS_CERT_PASSWORD', 'APPLE_ID', 'APPLE_APP_PASSWORD', 'APPLE_TEAM_ID'
]
```

## Was nach Erfüllung kommt

1. **MacOS-Secrets** (5/5) setzen → dann ist macOS-Pfad erfüllt. Ohne das hilft keine Windows-Entscheidung.
2. **Windows-Pfad** einen der beiden Wege komplettieren (ein Weg genügt, beide nicht):
   - **Pfad A (PFX):** `WINDOWS_CERT_BASE64` + `WINDOWS_CERT_PASSWORD` (selbst gekauftes Code-Signing-Zertifikat zum Base64-Encodieren).
   - **Pfad B (SignPath Foundation, kostenlos):** alle 4 `SIGNPATH_*` Secrets (nach Anmeldung + Projekt-Policy im Portal).
3. Sobald macOS + einer der Windows-Pfade komplett sind, lautet die Gate-Auswertung: **`READY`**.
4. Dann **manuelles Tag** erstellen: `git tag v0.66.0` (ohne Alpha-Suffix!), pushen, dann CI-Run (`ci.yml`) baut den Release. Beta/RC-Tags = Pre-releases.
5. Der Release-Build (`build-package.yml`) signiert dann automatisch (macOS + gewählter Windows-Pfad) und veröffentlicht.

## Hinweise

- **macOS ist Pflicht** für den offiziellen Gate. Es gibt kein Free-Tier; Apple Developer Program ($99/Jahr) nötig.
- **Windows-Pfad A** ist günstiger aber nicht kostenlos; **Pfad B SignPath** kostenlos, wenn akzeptiert (Anwendung aktuell abgelehnt, Oct 2026, siehe `docs/code-signing.md`).
- **Linux/Scoop etc.** brauchen kein Signing → fallen nicht an.
- **Workflow-Logik:** `build-package.yml` entscheidet: Wenn alle 4 `SIGNPATH_*` da → SignPath genutzt, PFX ignoriert. Sonst wenn beide `WINDOWS_CERT_*` da → signtool. Sonst unsigned.
- **Release-Trigger:** `Release (Alpha)` wird per `workflow_run` nach erfolgreichem CI ausgelöst, nicht beim Push.
- **Erster Beta-Tag:** `v0.66.0` (kein Alpha-Suffix).
- **Beta-Gate-Definition:** [CONTRIBUTING.md:101](CONTRIBUTING.md#L101): manuelles Tag `vX.Y.Z`, CI baut Release, Beta/RC-Tags = Pre-releases.

## Checkliste (nach Gate erfüllt)

- [ ] macOS Secrets (5) gesetzt
- [ ] Windows-Pfad A **od** B komplett (nicht beide)
- [ ] `GH_TOKEN="$(gh auth token)" python3 scripts/check-beta-gate.py` → `READY`
- [ ] `git tag v0.66.0` anlegen
- [ ] pushen (`git push origin v0.66.0`)
- [ ] CI läuft → `build-package.yml` signiert und veröffentlicht
- [ ] Release erscheint als Pre-release (`v0.66.0`)

## Links

- [Code-Signing-Doku: `docs/code-signing.md`](docs/code-signing.md) — alle 3 Pfade mit Schritt-für-Schritt
- [Release-Pipeline: `.github/workflows/build-package.yml`](.github/workflows/build-package.yml) — Secret-Definition + SignPath-Action
- [Beta-Gate-Skript: `scripts/check-beta-gate.py`](scripts/check-beta-gate.py) — Kriterien 1–6
