# cross-control landing page

The marketing/landing page for cross-control, served at
**https://cross-control.adjoint.uk**.

It is a single self-contained static file — `index.html` with all CSS and JS
inline, no build step, no external requests (CSP-locked). Edit it directly.

## Hosting — Cloudflare Pages

The page is served from Cloudflare Pages, connected to this repository. It
lives with the code so it stays in sync with what actually ships (a feature PR
updates the page in the same change).

One-time setup in the Cloudflare dashboard (admin):

1. **Workers & Pages → Create → Pages → Connect to Git** → pick
   `Adjoint-uk/cross-control`.
2. Build settings:
   - Framework preset: **None**
   - Build command: *(leave empty)*
   - Build output directory: **`site`**
   - Production branch: **`main`**
3. **Custom domains → Set up a custom domain →** `cross-control.adjoint.uk`
   (Cloudflare adds the DNS record automatically since adjoint.uk is on
   Cloudflare).

After that, every push to `main` redeploys automatically, and pull requests
get preview URLs.

## Files

- `index.html` — the page (edit here).
- `_headers` — Cloudflare Pages security + caching headers.
- `og.png` — 1200×630 social preview card (referenced from `index.html`'s
  `og:image` / `twitter:image`).
- `og.html` — source for the card. To regenerate `og.png` after editing it,
  screenshot it at 1200×630 with a headless browser, e.g.:

  ```js
  // node with playwright
  const { chromium } = require('playwright');
  const b = await chromium.launch();
  const p = await b.newPage({ viewport: { width: 1200, height: 630 }, colorScheme: 'dark' });
  await p.goto('file://' + require('path').resolve('site/og.html'));
  await p.screenshot({ path: 'site/og.png', clip: { x:0, y:0, width:1200, height:630 } });
  await b.close();
  ```
