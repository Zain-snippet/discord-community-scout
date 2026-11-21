# Setting up Google Drive sync

Sync is optional and off by default. Without it, Discord Community Scout is a
fully working local app and never contacts Google. With it, one JSON workspace
lives in your own Google Drive and multiple backends can reconcile through it.

**Status: not verified against Google in this repository.** Everything below is
a procedure for your own Google account. The code path (PKCE, token storage,
Drive `version`/`ETag`, three-way merge) is implemented and unit-tested, but no
real consent screen, token refresh or Drive write has been exercised here.

Read this first:

- **Google Drive stores data; it does not host the app.** A phone still needs a
  reachable backend (same Wi-Fi, Tailscale free plan, a temporary tunnel, or the
  optional Oracle Always Free VM).
- **Sign-in happens on the backend**, not in the browser. The browser never
  receives a token. Every browser that reaches the same backend shares the
  connection; a phone does not need its own Google login for that backend.
- **The app requests one scope:** `https://www.googleapis.com/auth/drive.appdata`.
  That is the app-specific folder only. No profile, Gmail, Photos, contacts or
  full-Drive access.
- **`appDataFolder` is hidden from the normal Drive UI.** You will not see
  `scout-data.json` in My Drive. That is expected; manage it through the app.

## What you need

- A Google account.
- The app running (`cargo run` locally, or the hosted HTTPS deployment).
- A browser on the same machine as the backend for the local sign-in.
- No payment method is needed for this scope. Do not enable paid Google Cloud
  products; the Drive API used here has a free quota that is far above personal
  use. Google can change quotas and policies, so check the console if something
  is refused.

## 1. Create a Google Cloud project

1. Open <https://console.cloud.google.com/>.
2. Create a project (for example `discord-community-scout`). A personal project
   is fine.
3. Select that project in the console header.

You do not need to create a VM, database, service account or API key.

## 2. Enable the Google Drive API

1. Go to **APIs & Services -> Library**.
2. Search for **Google Drive API**.
3. Open it and click **Enable**.

If this is skipped, sign-in can succeed but Drive requests fail with
`403 insufficientPermissions` or "Google Drive API has not been used".

## 3. Configure the OAuth consent screen

The exact labels move around between console revisions; look for "OAuth consent
screen" or "Branding"/"Audience" under **APIs & Services**.

1. Choose **External** (unless you use a Google Workspace organisation and want
   **Internal**).
2. Fill in the app name, a user support email and a developer contact email.
3. Add the scope `https://www.googleapis.com/auth/drive.appdata`. It may be
   shown as "View and manage its own configuration data in your Google Drive".
   Do not add broader Drive scopes.
4. Under **Test users**, add the Google account(s) that will sign in.
5. Leave the app in **Testing** while you try it.

### The 7-day refresh-token caveat

While the consent screen is in **Testing**, Google expires refresh tokens after
**7 days** and only listed test users can sign in. After a week the app will ask
you to sign in again; local data is untouched. This is a Google policy, not app
behaviour.

Publishing the app (even unverified, with the "unverified app" warning) avoids
that expiry, subject to Google's user caps and, for sensitive scopes, its
verification rules. Publishing is optional. Do not claim verification or remove
the warning unless Google has actually approved it.

## 4. Create the OAuth client

You need a different client type for local and hosted use.

### A. Local app (Desktop client)

1. **APIs & Services -> Credentials -> Create credentials -> OAuth client ID**.
2. Application type: **Desktop app**.
3. Name it (for example `Scout local`), then **Create**.
4. Copy the client ID and client secret. The secret is not a strong secret for
   installed apps, but still keep it out of Git and screenshots.

Desktop clients permit a loopback redirect, so no redirect URI needs to be
registered. The app builds:

```text
http://127.0.0.1:<PORT>/api/auth/google/callback
```

Use `127.0.0.1`, not `localhost`, and use the same `PORT` the app listens on
(default `8080`).

### B. Hosted app (Web application client)

1. **APIs & Services -> Credentials -> Create credentials -> OAuth client ID**.
2. Application type: **Web application**.
3. Under **Authorised redirect URIs**, add exactly:

   ```text
   https://<your-host>/api/auth/google/callback
   ```

   For the Oracle/DuckDNS deployment that is
   `https://scout-example.duckdns.org/api/auth/google/callback`. No trailing
   slash, no `http://`, no port unless you actually use one.
4. Leave **Authorised JavaScript origins** empty; the app does not run OAuth in
   browser JavaScript.
5. Copy the client ID and client secret.

`redirect_uri_mismatch` means this value and `SCOUT_PUBLIC_URL` disagree by even
one character (scheme, host, path, trailing slash).

## 5. Configure the app

### Locally

Create `.env` (ignored by Git) from `.env.example` and set:

```dotenv
PORT=8080
SCOUT_PUBLIC_URL=http://127.0.0.1:8080
GOOGLE_CLIENT_ID=1234567890-example.apps.googleusercontent.com
GOOGLE_CLIENT_SECRET=GOCSPX-example-secret
```

Restart the app. The startup log should say `Google Drive sync enabled` with the
redirect URI. If it says `disabled (GOOGLE_CLIENT_ID is not set)`, the variable
did not reach the process.

### On the hosted VM

See `docs/deployment.md`, section 9. In short, add to `deploy/.env`:

```dotenv
SCOUT_PUBLIC_URL=https://scout-example.duckdns.org
GOOGLE_CLIENT_ID=...
GOOGLE_CLIENT_SECRET=...
```

then re-run `./scripts/deploy.sh` and check
`docker compose ... logs app` for the enabled message.

Never commit `.env` or `deploy/.env`. Both are already git-ignored.

## 6. Sign in and verify

1. Open the app.
2. Click the sync pill in the sidebar ("Sync off" or "Sign in with Google").
3. Choose **Sign in with Google**, pick your account, and approve the
   `drive.appdata` scope.
4. You are redirected to `/?google=connected`.
5. The pill should reach **Synced** after the first upload. If records already
   existed locally, the app first asks how to combine them with Drive; nothing
   is uploaded or replaced until you choose.

What to check:

- `GET /api/auth/status` reports `google.signed_in: true`, `google.configured:
  true`, and a sync status other than `disconnected`.
- Create a record, wait for the pill to settle on **Synced**, and confirm the
  record survives a restart.
- If you have a second backend, sign in there with the same account and let it
  sync; records should appear on both after a sync (not instantly).
- The dialog's **Download a copy** returns the same document shape.

Do not mark sync as working in a report until these steps succeed on your own
account and, ideally, on two devices.

## 7. Multiple devices and phones

Each backend keeps its own SQLite cache and signs in separately. The shared
state is the Drive document.

- **Same backend, two browsers:** both see the same SQLite data immediately; no
  second Google login is needed.
- **Two backends (laptop + desktop):** each signs in once. Changes propagate on
  the next sync (startup, save debounce, 2-minute poll, or "Sync Now").
- **Phone:** the phone must reach a backend. Options, all free:
  - same Wi-Fi: `http://<laptop-lan-ip>:8080` (allow the port in the private
    Windows firewall);
  - Tailscale free plan: join the phone and laptop to the same tailnet;
  - a temporary Cloudflare quick tunnel (URL changes each run);
  - the optional Oracle Always Free VM with HTTPS.

Google Drive never makes the backend reachable by itself.

## 8. Token storage and logout

- **Windows:** tokens are encrypted with DPAPI in
  `data/google-tokens.bin`, bound to your Windows user.
- **Linux/macOS:** `data/google-tokens.json` with mode `0600` and a warning. It
  is not an OS keychain; protect the volume and the user account.
- **Docker/VM:** the file lives in the `app_data` volume under `/app/data`.
  Anyone who can read that volume can read the token; keep `deploy/.env` and the
  volume restricted.

**Logout** revokes the refresh token at Google, deletes the local credential and
sets the status to disconnected. It does **not** delete local records or the
Drive document. To revoke from Google's side as well, visit
<https://myaccount.google.com/permissions> and remove the app.

To disable sync without deleting anything: unset `GOOGLE_CLIENT_ID` (and the
secret) and restart. The app behaves as a purely local app again.

## 9. Suggested verification order

1. Drive API enabled.
2. Consent screen has the `drive.appdata` scope and your account as a test user.
3. Redirect URI/client type matches the host (Desktop for loopback, Web for
   HTTPS).
4. `SCOUT_PUBLIC_URL` matches exactly.
5. Click sign-in and finish consent.
6. Wait for **Synced**, then restart and confirm the data.
7. Only then try a second device.

## 10. Troubleshooting

| Symptom | Usual cause | Fix |
| --- | --- | --- |
| "Google sync is not configured" | `GOOGLE_CLIENT_ID` empty or not passed to the process/container | Set it in `.env`/`deploy/.env`, restart, check the startup log |
| `redirect_uri_mismatch` | Redirect URI and `SCOUT_PUBLIC_URL` differ | Make them identical; register the exact URI on a **Web** client |
| "Access blocked" / app not verified | Account not a test user, or consent screen in Testing | Add the account under Test users, or publish deliberately |
| `403 insufficientPermissions` | Drive API not enabled, or wrong scope | Enable **Google Drive API**, keep only `drive.appdata` |
| Sign-in works, then asks again after ~7 days | Testing-mode refresh-token expiry | Expected; publish the app or sign in again |
| `invalid_grant` on refresh | Token revoked, expired, or password changed | Sign in again; logout first if needed |
| Status stays **Auth expired** | Refresh failed | Click sign-in; local data is safe |
| Sync stays **Offline** | Backend cannot reach Google, or laptop asleep (for a phone) | Check outbound HTTPS/DNS; the app keeps changes locally |
| Data file not visible in Drive | `appDataFolder` is hidden by design | Use the app/API; do not expect it in My Drive |
| Two devices disagree on one field | Both changed the same field | Resolve in the conflict dialog; both values are preserved until then |

## 11. What this setup does not do

- It does not host the app, serve the API, or make a phone reach a sleeping
  laptop.
- It does not encrypt data end-to-end. Google can read the Drive file; the app
  does not claim otherwise. Do not store secrets in community notes.
- It does not sync instantly. Two backends converge on the next poll or manual
  sync.
- It does not replace Basic Auth on the hosted deployment. Keep that enabled.
- It does not use the Google Drive UI for recovery. The app keeps five local
  snapshots and offers JSON export; Drive's own file history is a separate
  fallback.
