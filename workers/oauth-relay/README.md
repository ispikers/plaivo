# plaivo-oauth-relay

A Cloudflare Worker that signs Plaivo into MyAnimeList. Plaivo can't keep a client secret (anything in the binary can be extracted), and a TV can't receive a browser redirect, so the relay holds the secrets, receives the provider redirect, swaps the code for tokens and hands them to the app that polls for them. Trakt doesn't use it: it signs in with the device-code flow and, like every new Trakt app, has no client secret.

The Worker rate-limits by client IP and caps request bodies, then hands each request to one SQLite-backed Durable Object (`Sessions`) that runs the relay logic in `src/relay.ts`. A Worker instance keeps no state between requests, so sessions are stored there. A session expires 10 minutes after it starts or after the provider redirect, at most 65 536 are held at once, and tokens are handed out once and then deleted. Tokens are never logged.

## Flow

1. The app calls `POST /auth/start` and shows `authorize_url` with its QR code. It is the relay's short link `/auth/{service}?state=…`, which redirects (302) to the provider's long authorize URL while the session is open, so the QR code stays small.
2. The user approves; the provider redirects to `/auth/{service}/callback`. The relay checks `state`, exchanges the code and shows a "you can close this tab" page.
3. The app polls `GET /auth/result?session=…` every few seconds until it gets 200 (tokens), 403 (cancelled or failed) or 410 (expired).

## API

JSON bodies are UTF-8, at most 4 KiB; larger bodies get 413 `body_too_large`. More than 50 requests a minute from one IP get 429 `rate_limited`. Every response carries `Cache-Control: no-store`. `service` is `"mal"`. Errors are `{"error": "<code>"}`.

`POST /auth/start` with `{"service": "mal"}`

| Status | Body                                                                                                       |
| ------ | ---------------------------------------------------------------------------------------------------------- |
| 200    | `{"session": "<64 hex>", "authorize_url": "https://relay.plaivo.app/auth/mal?state=…", "expires_in": 600}` |
| 400    | `invalid_service`                                                                                          |
| 503    | `busy` when 65 536 sessions are pending                                                                    |

`GET /auth/result?session=<id>`

| Status | Body                                                                                                                                                                                          |
| ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 204    | (empty) still waiting for the user                                                                                                                                                            |
| 200    | `{"access_token": "…", "refresh_token": "…", "expires_in": 2678400, "token_type": "Bearer"}`, returned once; fields other than `access_token` are omitted when the provider doesn't send them |
| 403    | `access_denied` (user cancelled) or `exchange_failed` (provider refused the code), returned once                                                                                              |
| 410    | `expired`: unknown, expired or already collected                                                                                                                                              |
| 400    | `invalid_session`: parameter missing                                                                                                                                                          |

`POST /auth/refresh` with `{"service": "mal", "refresh_token": "…"}`

| Status | Body                                                                                                                |
| ------ | ------------------------------------------------------------------------------------------------------------------- |
| 200    | the token JSON above                                                                                                |
| 400    | `invalid_request` or `invalid_service`                                                                              |
| 401    | `refresh_rejected`: MAL answered 400 or 401, so the refresh token is no longer valid; sign in again                 |
| 502    | `provider_unavailable`: network error, 5xx or any other 4xx (e.g. 403, 429); keep the refresh token and retry later |

`GET /auth/{service}?state=…` redirects to the provider while the session is waiting, otherwise it shows an "expired" page (400).

`GET /auth/{service}/callback` is for the provider only. It returns an HTML page: 200 on success or cancel (`error=access_denied`), 400 for an unknown, expired or reused `state`, 502 when the token exchange fails or the provider redirects with any other `error`.

## Configuration

| Name                       | Kind                      | Meaning                                                                       |
| -------------------------- | ------------------------- | ----------------------------------------------------------------------------- |
| `PLAIVO_RELAY_PUBLIC_URL`  | var in `wrangler.jsonc`   | Public base URL, `https://relay.plaivo.app`. Redirect URIs are built from it. |
| `PLAIVO_MAL_CLIENT_ID`     | secret, for MAL           | MyAnimeList client ID.                                                        |
| `PLAIVO_MAL_CLIENT_SECRET` | secret, if MAL issued one | MyAnimeList client secret.                                                    |

`PLAIVO_MAL_CLIENT_ID` must be set; otherwise every request fails with a 500 and `wrangler tail` shows why.

Plaivo uses `https://relay.plaivo.app` unless built with `PLAIVO_OAUTH_RELAY` set to another URL (e.g. a local `wrangler dev`).

## Registering the apps

The redirect URI must match exactly, including scheme and no trailing slash.

**MyAnimeList:** <https://myanimelist.net/apiconfig> → Create ID. App Type `web` (it gets a secret; `other` works too and has none). App Redirect URL `https://relay.plaivo.app/auth/mal/callback`. Fill in the name, description and homepage, then copy the Client ID (and Client Secret). MAL uses PKCE with the `plain` method, which the relay handles.


## Deploying

The `plaivo.app` zone must be on Cloudflare (DNS managed there); `wrangler deploy` then creates the `relay.plaivo.app` custom domain and its certificate. Everything fits the Workers Free plan.

```sh
cd workers/oauth-relay
bun install
bun wrangler login
bun wrangler secret put PLAIVO_MAL_CLIENT_ID
bun wrangler secret put PLAIVO_MAL_CLIENT_SECRET
bun run deploy
```

Check it: `curl -X POST https://relay.plaivo.app/auth/start -d '{"service":"mal"}'` should return a session and an authorize URL. `bun wrangler tail` streams live requests and the token-exchange errors; Workers Logs (persistent observability) stay off because they would store query strings, which carry the one-time `code`, `state` and session id.

## Development

- `bun test` runs the routes against a fake token exchange and an in-memory SQLite database.
- `bun check` type-checks.
- To poke it by hand: `bun wrangler dev --var PLAIVO_MAL_CLIENT_ID:x --var PLAIVO_RELAY_PUBLIC_URL:http://127.0.0.1:8787`.

AniList isn't supported: it manually blocks Cloudflare Worker IPs, so the Worker can't exchange its codes, and it has turned off the implicit grant.
