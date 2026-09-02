# Stream Alerts (Follow/Sub/Donation Overlays)

Rivulet does not host alert animations itself. Like OBS, you point a
**browser source** at the **widget URL** of an alert service, and that
service renders the follow/sub/donation animations on top of your scene.

Supported providers (any https widget URL works, these two are guided):

| Provider       | Widget URL shape                              | Where to find the token                     |
|----------------|-----------------------------------------------|---------------------------------------------|
| Streamlabs     | `https://streamlabs.com/alert-box/v2/<token>` | Alert Box → Settings → **Widget URL**       |
| StreamElements | `https://streamelements.com/overlay/<token>`  | Overlays → your overlay → **Copy URL**      |
| Custom         | any https URL                                 | paste it verbatim (e.g. own hosted overlay) |

## Importing an alert overlay

1. Open the **Scenes** view (browser source panel).
2. Under **Alert overlay import**, pick the provider (**Streamlabs**,
   **StreamElements**, or **Custom URL**).
3. Paste the **widget token** from your dashboard (for Streamlabs /
   StreamElements) or the full https URL (for Custom).
4. Click **Import overlay** — the widget URL is built, validated, and loaded
   into the browser source. The token is cleared immediately and is never
   logged.

The provider dropdown and token field are purely import helpers: they build
the URL and hand it to the existing browser source. You can still edit the
URL manually afterwards (Apply URL), or paste any other https widget URL
directly.

> **Tip:** after importing, disable **Allow interaction** on the browser
> source so the alert overlay never steals pointer/keyboard focus.

## Privacy

The token is only used to construct the widget URL at import time and is
cleared from the input fields immediately after. It is never written to the
logs, the scene model keeps only the final https URL (the token is part of
it, exactly as in OBS), and no alert data leaves the app beyond what the
widget service itself loads.

## Validation rules

- Only **https** widget URLs are accepted (overlay scripts depend on the
  service's secure backend).
- Tokens must be non-empty and free of whitespace and URL-dangerous
  characters (`/ ? # & = " '`). The services validate the token when the
  widget loads — Rivulet only checks the shape.
- A pasted URL is used to **guess** the provider for the dropdown, so you
  can switch providers without retyping.

## Tests

- `rivulet-core/src/alerts.rs`: URL shapes per provider, provider guessing,
  https/whitespace validation, token validation, end-to-end
  `build_overlay_url` cases, and a test that the generated URLs pass the
  browser-source URL validator.
- `rivulet-gui/src/app.rs`: import behavior tests (Streamlabs,
  StreamElements, Custom), and that invalid input leaves the browser source
  untouched and surfaces an error.
- `ci_pinning.rs`: guard keeping the import wired, localized (DE/EN), and
  documented.

## Roadmap

Alert import is part of the M5 community dock (see
`docs/obs-vision-roadmap.md`). A future item is native alert hosting
(platform EventSub → Rivulet-rendered overlay) instead of third-party
widgets.
