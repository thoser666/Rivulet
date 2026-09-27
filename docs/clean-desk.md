# Clean-Desk-Konzept — Solo → Team-Übergang

> DE-first (the maintainer's working language); an English checklist summary
> lives at the bottom. This document is the working agreement for account,
> workspace and session hygiene that multi-person operation requires. The
> team-mode flip itself is a runbook:
> [`docs/team-onboarding-runbook.md`](team-onboarding-runbook.md).

## Warum Clean Desk hier zählt

Rivulet vertreibt signierte Binaries und published automatisch Alpha-Releases
aus `develop`. Der wertvollste Angriffspunkt ist deshalb nicht das Repository
(Rulesets, Pinning-Guards und SHA-Pflicht decken das ab), sondern **der
Maintainer-Account**: Wer eine aktive Session oder einen PAT klaut, erbt
exakt die Rechte, die die Rulesets dem Account legitimate geben — inklusive
Ruleset-Änderung. Clean Desk heißt hier: die Angriffsfläche *um den Account*
so klein halten, dass ein Team-Zugang nicht eine Tür in einer offenen Halle
ist.

## Grundsätze

1. **Least privilege pro Token.** Kein langlebiger Classic-PAT mit `repo`.
   Neue Tokens sind fine-grained, mit Ablaufdatum und auf den Zweck
   beschränkt (z. B. nur `contents:write` für einen Automations-Job, nur
   `metadata:read` für einen Analyzer).
2. **Keine geteilten Credentials.** Jede Person/ein jeder Automations-Job
   hat eigene Zugänge. Ein geteiltes Passwort macht Session-Audits
   wertlos.
3. **Zwei-Personen-Regel für kritische Pfade.** Sobald ein zweiter
   Maintainer existiert, merged niemand allein in kritische Pfade
   (CODEOWNERS + team-mode Ruleset). Bis dahin gelten die automatisierten
   Checks als Review (siehe `CONTRIBUTING.md`).
4. **Maschine ist Teil des Trust-Chain.** Festplattenverschlüsselung an,
   OS-Updates aktuell, kein Klartext-Token in Dateien, die synchronisiert
   werden (Dotfiles-Repos, Notizen, Chat-Verlauf).

## Account-Hygiene (monatlich prüfen)

- [ ] Passkey/2FA aktiv — Passkey bevorzugt, TOTP als Fallback, kein SMS.
- [ ] `Settings → Sessions`: unbekannte oder alte Sessions widerrufen.
- [ ] `Settings → Developer settings → Personal access tokens`: jede Token
      prüfen — Ablaufdatum in der Zukunft? Scope minimal? Ungenutzt →
      löschen. Der eigene `gh`-CLI-Token gehört dazu.
- [ ] `Settings → Emails`: keine Wegwerf-Adresse als Fallback.
- [ ] Dependabot-Alerts (`security/dependabot/N`) abgearbeitet — offene
      Alerts auf dem Default-Branch sind auch Clean Desk.
- [ ] `Settings → Authorized OAuth Apps`: Apps ohne aktiven Nutzwert
      widerrufen.

## Workspace-Hygiene

- [ ] `gh auth status` vor Arbeitsbeginn auf fremden Maschinen; danach
      `gh auth logout`.
- [ ] Tokens nie in Shell-History (`export GH_TOKEN=…` vermeiden —
      `gh auth login` nutzt den Keyring).
- [ ] `.gitignore` deckt Build-Artefakte und lokale Config; keine
      Aufzeichnungs-/Test-Dateien mit persönlichem Pfad committen
      (`C:/Users/<name>/…` bleibt in Logs auf dem eigenen Rechner).
- [ ] Git-Identity im Repo-Flow: die eingetragene E-Mail ist die, an die
      der Account gebunden ist (`git -c user.email=…` im Flow-Command).

## Repository-Hygiene (bereits eingerichtet)

- `develop`-Ruleset ohne Bypass-Actors, PR-Pflicht, Required Checks.
- `release-tags-protected` / `release-branches-protected` Rulesets.
- `sha_pinning_required` auf Actions-Ebene, Actions auf volle SHAs gepinnt,
  ci_pinning-Tests als Content-Guards.
- `GITHUB_TOKEN` auf read; Secret Scanning + Push Protection aktiv.
- Ruleset-Guard-Workflow verifiziert die Ruleset-Konfiguration kontinuierlich
  (`scripts/check-develop-ruleset.py`).

## Übergangs-Checkliste Solo → Team

Der Wechsel ist kein Settings-Klick, sondern ein Runbook:

1. [ ] Zweite Person hat Access (Collaborator/Team) —**nur** diese Person
      hinzufügen, noch keine Ruleset-Änderung.
2. [ ] CODEOWNERS-Default-Owner auf das Maintainer-Team umstellen
      (Runbook Schritt 2).
3. [ ] `docs/security.md` und `CONTRIBUTING.md`: die Single-Maintainer-
      Formulierungen sind im selben PR aktualisiert wie der Flip.
4. [ ] Ruleset-Flip im Runbook ausgeführt (`required_approving_review_count:
      1`) **und** `check-develop-ruleset.py --team-mode` zum Guard gemacht.
5. [ ] Erste gemeinsame Woche: beide mergen je einen PR über den neuen
      Gate; danach gilt die Zwei-Personen-Regel als normal.

## English summary (checklist)

- **Goal:** shrink the attack surface *around* the maintainer account — the
  repo-side hardening (rulesets, SHA pinning, content guards) is in place;
  a stolen session is the realistic backdoor.
- **Monthly:** audit sessions + PATs (fine-grained, expiring, minimal),
  passkey/2FA on, Dependabot alerts worked off, OAuth apps pruned.
- **Workspace:** `gh auth logout` on shared machines, no tokens in shell
  history or synced files, encrypted disks.
- **Team transition:** follow `docs/team-onboarding-runbook.md` — access
  first, CODEOWNERS flip second, docs in the same PR, ruleset
  `required_approving_review_count: 1` + `--team-mode` guard last.
