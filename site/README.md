# Upleveler website

The public landing page at [upleveler.dev](https://upleveler.dev). A small Rust program renders it to static HTML, and
Cloudflare serves the files as a [static-assets Worker](https://developers.cloudflare.com/workers/static-assets/):
there is no server code and no user data.

Your work logs are never on this website. The personal dashboard runs on your own
computer, from the `upleveler` app.

## Build

From the repository root:

```sh
cargo run -p upleveler-site            # writes site/dist/
cargo run -p upleveler-site -- out/    # or another directory
```

Open `site/dist/index.html` in a browser, or preview it the way Cloudflare serves it
(needs [Node.js](https://nodejs.org)):

```sh
cd site
npx wrangler dev
```

## Deploy

The [Site workflow](../.github/workflows/site.yml) builds and deploys automatically:

| Event | Result |
|---|---|
| Push to `main` that touches `site/` | Deploys to production at `upleveler.dev` |
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
| `src/main.rs` | Renders every page with [maud](https://maud.lambda.xyz) |
| `wrangler.jsonc` | Cloudflare configuration: Worker name, the `upleveler.dev` domain and the `dist/` directory |
| `dist/` | Build output, not committed |
