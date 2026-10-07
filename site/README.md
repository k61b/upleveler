# Upleveler website

The public landing page at [upleveler.dev](https://upleveler.dev). A small Rust program renders it to static HTML, and
Cloudflare serves the files as a [static-assets Worker](https://developers.cloudflare.com/workers/static-assets/):
there is no server code and no user data.

Your work logs are never on this website. The personal dashboard runs on your own
computer, from the `upleveler` app (`upleveler web`).

The site and the dashboard share one design system: the colour tokens, the mark,
the Manrope font and the components all come from `crates/upleveler/src/web/`, so
the site never defines its own colours or styles. `cargo test` checks contrast and
lints the site's sources together with the dashboard's.

The page has a hero with a terminal session, the three steps (log, gap, brag),
people and goals (notes, goals and 1:1 preparation), a live demo, a privacy
section and the install commands. The demo is not a
screenshot: it is the real dashboard Overview (`views::demo_frame`) rendered with
the made-up data in `crates/upleveler/src/web/demo.rs`.

## Build

The site needs Rust 1.85 or newer (the app itself still builds on 1.82). From the
repository root:

```sh
cargo run -p upleveler-site            # writes site/dist/
cargo run -p upleveler-site -- out/    # or another directory
```

Pages use root paths (`/assets/style.css`), so serve the folder instead of opening
the files directly. Preview it the way Cloudflare serves it (needs
[Node.js](https://nodejs.org)):

```sh
cd site
npx wrangler dev
```

or with any static server, for example `python3 -m http.server -d site/dist`.

Besides the pages, the build writes:

| File | What it is |
|---|---|
| `og.png` | The 1200 × 630 link preview, rendered from the tokens and the mark |
| `favicon.svg`, `apple-touch-icon.png`, `icon-*.png`, `site.webmanifest` | Icons drawn from the mark |
| `sitemap.xml`, `robots.txt` | For search engines |
| `install.sh` | The installer behind `curl -fsSL https://upleveler.dev/install.sh \| sh`, copied from `site/install.sh`; it installs the latest GitHub release |
| `gallery/` | Every component on both surfaces, for design review. Not linked, `noindex`, excluded in `robots.txt` |

PNGs are rendered with [resvg](https://github.com/linebender/resvg) and the bundled
Manrope TTF, so they come out the same on every machine. Generated files are not
committed.

## Deploy

The [Site workflow](../.github/workflows/site.yml) builds and deploys automatically:

| Event | Result |
|---|---|
| Push to `main` that touches `site/` or `crates/upleveler/src/web/` | Deploys to production at `upleveler.dev` |
| Pull request from this repository | Uploads a preview version and prints its URL |
| Pull request from a fork | Builds only |

Deploying needs two repository secrets (**Settings → Secrets and variables → Actions**):

- `CLOUDFLARE_API_TOKEN`: an API token created from the **Edit Cloudflare Workers** template
- `CLOUDFLARE_ACCOUNT_ID`: shown on the Workers & Pages overview in the Cloudflare dashboard

Without them the workflow only builds the site. The workflow pins the Wrangler version
(`WRANGLER_VERSION` in `site.yml`), because the action otherwise installs Wrangler 3,
which cannot read `wrangler.jsonc`.

To deploy by hand instead:

```sh
cargo run -p upleveler-site
cd site
npx wrangler login
npx wrangler deploy
```

## Files

| Path | What it is |
|---|---|
| `src/main.rs` | Renders the pages with [maud](https://maud.lambda.xyz) and copies the shared assets (styles, script, fonts) into `dist/` |
| `src/og.rs` | Renders the link preview and the app icons to PNG |
| `install.sh` | The macOS and Linux installer: picks the archive for the system, checks it against the release's `SHA256SUMS`, installs to `~/.local/bin` |
| `assets/fonts/` | Manrope as TTF for the PNG renderer (SIL Open Font License) |
| `wrangler.jsonc` | Cloudflare configuration: Worker name, the `upleveler.dev` domain and the `dist/` directory |
| `dist/` | Build output, not committed |
