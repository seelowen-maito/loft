# Loft showcase

Two things to share Loft with friends, built from `tools/` (see Rebuilding; the output isn't committed):

- **`index.html`**: a single web page with everything inlined, so it opens offline from a double-click or on any static host. It has scroll animations, device mockups and a light/dark switch, and it works on phones and desktops.
- **`loft-showcase.pdf`**: the same story as an 11-page landscape overview.

Every screenshot uses made-up data. The web shots come from a separate demo Loft (bakery shop, weather app and recipe blog projects). The iPhone shots come from the app's built-in demo data (`-ui-testing`), taken in the iOS Simulator.

## Rebuilding

The tools are in `tools/`. Change the words in `content.py`.

1. Demo backend: start Loft with an empty `LOFT_ROOT`, `LOFT_PORT=7440` and `claude_bin` set to `/usr/bin/false` in its settings, so no agent actually runs. Then run `python3 seed_demo.py <root>/.loft/loft.db`.
2. Web screenshots: run `node shoot_web.js '<agent ids json>' '<task ids json>' shots/web`. It needs Playwright and writes dark-only shots, because the web app is dark only.
3. iPhone screenshots: take them in the Simulator with `-ui-testing`. Put them in `shots/ios/{light,dark}/<key>.png`, with the keys from `PHONE_CAPTIONS`.
4. Build: run `python3 build.py shots out`, then `node pdf.js out/print.html out/loft-showcase.pdf`. It needs `cwebp`. The headline font (Instrument Serif, SIL Open Font License) is in `fonts/` and gets inlined, so the page makes no outside requests.
5. Check: `node selfcheck.js out/index.html <dir>` loads the page offline at desktop, phone and small-phone widths, with and without reduced motion. It reports errors, outside requests, broken images, overflow, content that never appears, and whether the theme toggle works.
