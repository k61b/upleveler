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

## Build

From the repository root:

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

The build writes `/gallery/`, a page with every component on both surfaces for
design review. It is not linked, has `noindex` and is excluded in `robots.txt`.

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
| `src/main.rs` | Renders the pages with [maud](https://maud.lambda.xyz) and copies the shared assets (styles, script, fonts, favicon) into `dist/` |
| `wrangler.jsonc` | Cloudflare configuration: Worker name, the `upleveler.dev` domain and the `dist/` directory |
| `dist/` | Build output, not committed |
