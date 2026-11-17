# Deploying to Oracle Cloud Always Free

**Status: prepared, not deployed.** No Oracle Cloud account, VM, hostname or
public URL exists yet. Everything in this document is a procedure to follow on
your own account; nothing here is a report of a live deployment.

What *has* been checked in the repository (2026-11-17):

| Check | Result |
| --- | --- |
| `bash -n` on every script in `scripts/` | pass |
| `docker compose config` (Compose v2.29.7) with a real `deploy/.env` | pass, default and `duckdns` profiles |
| `caddy validate` (Caddy 2.8.4) on `deploy/Caddyfile` | pass |
| App build/test/clippy/fmt/release build | pass (see README) |

What has **not** been run: an ARM64 container build, a real VM, DNS, certificate
issuance, HTTPS access, backup/restore against a live volume. Do not treat the
deployment as complete until the verification steps in section 11 and the
restore test in section 12 have actually succeeded.

---

## 1. What gets deployed

```
                    Internet
                       |
              :80  (ACME + redirect)      :443 (HTTPS, HTTP/3)
                       |                       |
                 +-----v-----------------------v-----+
                 |  caddy:2.8-alpine                 |
                 |  TLS, HSTS, Basic Auth (bcrypt)  |
                 |  volumes: caddy_data, caddy_config|
                 +-----------------+-----------------+
                                   | internal Docker network
                          +--------v---------+
                          |  app (Rust/Axum) |
                          |  :8080, not      |
                          |  published       |
                          |  volume: app_data|
                          |  /app/data/scout.db
                          +------------------+
```

- Only Caddy publishes ports **80/443**. The app has no `ports:` mapping.
- The SQLite file lives in the named volume `discord-community-scout_app_data`,
  mounted at `/app/data`. It is never served as a static file (Axum serves only
  `static/`).
- Caddy state (certificates, ACME account) lives in `caddy_data` and
  `caddy_config`, so certificates survive container/VM restarts.
- `restart: unless-stopped` on every service plus Docker enabled at boot.
- Optional `duckdns` profile keeps a DuckDNS A record pointed at the VM.

Files:

| Path | Purpose |
| --- | --- |
| `Dockerfile` | multi-stage build, unchanged architecture, `/app/data` volume, healthcheck |
| `deploy/docker-compose.yml` | app + Caddy + optional DuckDNS |
| `deploy/Caddyfile` | TLS, HTTP->HTTPS redirect, Basic Auth, security headers |
| `deploy/.env.example` | template; copy to `deploy/.env` (git-ignored) |
| `scripts/install-vm.sh` | Docker Engine, Compose plugin, sqlite3, ufw, unattended upgrades |
| `scripts/deploy.sh` | validates config, builds, starts, waits for health |
| `scripts/install-backup-timer.sh` | daily systemd timer for backups |
| `scripts/backup-db.sh` | consistent copy + integrity check + compression + retention |
| `scripts/verify-backup.sh` | checksum, `integrity_check`, `foreign_key_check`, schema, counts |
| `scripts/restore-db.sh` | verified, rollback-safe restore |
| `scripts/verify-deployment.sh` | external HTTPS/auth/workflow checks |

---

## 2. Always Free limits, and staying at $0

Use **only** resources labelled **Always Free**. Do not upgrade the tenancy to
Pay As You Go for this deployment, and do not create any paid resource.

The relevant Always Free entitlements (verify the current values in the Oracle
console before creating anything - Oracle can change them):

| Resource | Always Free entitlement | Used here |
| --- | --- | --- |
| Ampere A1 compute | up to **4 OCPUs / 24 GB RAM total** per tenancy (one VM is fine) | one VM, normally 4 OCPU / 24 GB |
| AMD micro compute | 2 x `VM.Standard.E2.1.Micro` (1/8 OCPU, 1 GB each) | not used |
| Block volume | **200 GB total**, 5 volume backups | 50-100 GB boot volume |
| Object Storage | roughly **20 GB total** across tiers, plus limited requests | optional off-site backups |
| Outbound transfer | generous monthly allowance (10 TB/month historically) | normal browsing/API use |
| Load balancer | 1 flexible LB (10 Mbps) | **not used** - Caddy runs on the VM |
| Managed database | none on Always Free | **not used** - SQLite in the volume |
| Container registry / monitoring | paid | **not used** - image builds on the VM |

Cost safeguards in this setup:

- No paid load balancer, no managed database, no container registry, no paid
  monitoring/APM, no NAT gateway, no extra VNICs.
- One VM inside the A1 allowance, one boot volume inside the 200 GB allowance.
- The image is built locally on the VM; nothing is pushed to a registry.
- Caddy obtains free certificates from Let's Encrypt (ACME).
- DuckDNS is free (no card).
- The app makes one outbound request family: GitHub raw content for the
  curated list, cached in memory. No paid API is called.

### 30-day trial credits vs Always Free

A new Oracle Cloud account usually starts with a **30-day trial** and a credit
balance (historically US$300). Trial credits can pay for larger/paid shapes and
services, but they expire after the trial. **Always Free resources are separate
from the credits**: they remain available after the trial ends, as long as the
account is in good standing and the resources stay within the Always Free
limits.

Signing up may require a payment method for identity verification. That does
not make Always Free resources billable, but Oracle may place a temporary
verification hold. This project does not need your card details and you should
never send them to anyone. If the signup flow asks for payment information, it
is Oracle asking you directly - decide there, and do not upgrade to Pay As You
Go for this deployment.

At the end of the trial, paid resources (if any) can be stopped or reclaimed.
Only create Always Free shapes so there is nothing paid to reclaim.

### Idle-instance reclamation

Oracle's Always Free terms allow Oracle to reclaim **idle** compute instances.
Oracle's published idle definition (check the current wording in the Always Free
FAQ) is roughly: 95th-percentile CPU utilisation below 10%, network utilisation
below 10%, and memory utilisation below 10%, measured over a 7-day window.
Instances that look idle may be stopped or terminated.

What this means:

- A small app with occasional human use can look idle. That is a real risk.
- **Do not fake activity** (busy loops, artificial CPU/network load) to dodge
  the policy. That is exactly the kind of abuse the policy targets, and it is
  not implemented here.
- Keep **off-site** backups (section 12), so a reclaimed VM is an inconvenience,
  not data loss.
- If the VM is stopped/reclaimed, rebuild it from this guide and restore the
  latest backup. A genuinely used instance is less likely to be flagged.

---

## 3. Prerequisites on Windows

You need:

- An Oracle Cloud account (section 4).
- PowerShell 5.1 or PowerShell 7 (the commands below work in both).
- `ssh` and `ssh-keygen`, included with Windows 10/11.
- `git` for Windows if you want to clone locally (optional - the VM clones).

Create an SSH key pair **on your PC**. Never share the private key:

```powershell
ssh-keygen -t ed25519 -C "scout-vm" -f "$env:USERPROFILE\.ssh\scout-vm"
```

- Public key: `$env:USERPROFILE\.ssh\scout-vm.pub` - uploaded to Oracle.
- Private key: `$env:USERPROFILE\.ssh\scout-vm` - stays on your PC.

Restrict the private key's permissions (OpenSSH refuses overly open keys):

```powershell
icacls "$env:USERPROFILE\.ssh\scout-vm" /inheritance:r /grant:r "$($env:USERNAME):(R)"
```

---

## 4. Create the Oracle Cloud account

1. Open <https://signup.oraclecloud.com/> (or "Start for free" from
   <https://www.oracle.com/cloud/free/>).
2. Enter your country/email, verify the email and phone number.
3. Choose a **home region**. This cannot be changed later for the tenancy.
   - Prefer **Hyderabad (`ap-hyderabad-1`)** or **Mumbai (`ap-mumbai-1`)** if
     they are offered and close to you.
   - Ampere A1 capacity and Always Free service availability vary by region.
4. Complete identity verification. If a payment method is requested, it is for
   verification only; do not upgrade to Pay As You Go.
5. Wait for the "tenancy is ready" email, then sign in to the console:
   <https://cloud.oracle.com/>.

If signup or region selection fails, that is an Oracle-side blocker; report it
rather than choosing a paid region or shape.

---

## 5. Create the Always Free VM

In the Oracle console:

1. **Compute -> Instances -> Create instance**.
2. Name: `scout-vm`.
3. **Image and shape**:
   - Image: **Canonical Ubuntu 24.04** (aarch64/ARM build; "Ubuntu 24.04
     Minimal aarch64" is fine).
   - Shape: **Ampere -> VM.Standard.A1.Flex**.
   - OCPUs: **4**, Memory: **24 GB** (the full Always Free A1 allowance).
   - If the console reports **"Out of host capacity"**, reduce to 2 OCPU / 12 GB
     and retry, try another availability domain, or retry later. Do **not**
     switch to a paid shape. If no free A1 capacity is available at all, stop
     and report it.
4. **Networking**: create/select a VCN with a public subnet. Ensure
   **Assign a public IPv4 address** is set to **Yes**.
5. **Add SSH keys**: choose "Upload public key file" and select
   `$env:USERPROFILE\.ssh\scout-vm.pub`.
6. **Boot volume**: 50 GB (or up to 100 GB) is enough and stays inside the
   200 GB Always Free block-volume allowance. Leave encryption defaults on.
7. Create the instance and wait for the **Running** state. Note the
   **Public IP address** on the instance page.

The default public IP is ephemeral: it can change if the instance is stopped
and started. That is why the DuckDNS updater is included.

### Open the cloud firewall (Security List)

Oracle blocks everything except SSH by default at the VCN level.

1. **Networking -> Virtual Cloud Networks ->** your VCN **-> Security Lists ->
   Default Security List**.
2. **Add Ingress Rules**:

| Stateless | Source | IP Protocol | Destination Port | Purpose |
| --- | --- | --- | --- | --- |
| No | `0.0.0.0/0` | TCP | `80` | ACME HTTP challenge + HTTPS redirect |
| No | `0.0.0.0/0` | TCP | `443` | HTTPS (and HTTP/3 over UDP 443) |
| No | *your public IP* `/32` | TCP | `22` | SSH - prefer restricting this |

Leave the default egress rule (allow all) so the VM can reach Docker Hub,
Let's Encrypt and GitHub.

Do **not** open port 8080 or any database port. The app port is internal only.

---

## 6. Connect from Windows

```powershell
ssh -i "$env:USERPROFILE\.ssh\scout-vm" ubuntu@<PUBLIC_IP>
```

Accept the host key fingerprint on first connection. If SSH fails:

- Confirm the instance is Running and you are using the right public IP.
- Confirm the Security List has TCP 22 from your current public IP
  (`curl.exe https://ifconfig.me/ip`).
- Confirm you uploaded the **public** key and are using the matching private key.

You do **not** need to send anyone your private key, Oracle password, or
payment details.

---

## 7. Install Docker and the host firewall

On the VM:

```bash
sudo apt-get update
sudo apt-get install -y git
git clone https://github.com/Zain-snippet/discord-community-scout.git
cd discord-community-scout
sudo ./scripts/install-vm.sh
```

What the script does:

- installs Docker Engine + Compose plugin from Docker's apt repository
  (native `arm64` packages), enables Docker at boot;
- installs `sqlite3` (used by the backup scripts) and unattended security
  upgrades;
- configures `ufw` to allow only your SSH port, 80 and 443;
- backs up Oracle's default `/etc/iptables/rules.v4` to
  `/etc/iptables/rules.v4.pre-scout` and disables `netfilter-persistent`, which
  would otherwise re-apply Oracle's "reject everything" rules at boot.

If your SSH port is not 22, pass it explicitly:
`sudo ./scripts/install-vm.sh --ssh-port 2222`. Run `--help` for options.

Log out and back in (or run `newgrp docker`) so the `ubuntu` user can use Docker
without `sudo`.

---

## 8. DNS: free DuckDNS hostname

1. Sign in at <https://www.duckdns.org/> (GitHub/Google/Reddit/Twitter login).
2. Create a subdomain, for example `scout-example` -> the hostname becomes
   `scout-example.duckdns.org`.
3. Copy the **token** shown at the top of the page. Treat it as a secret: do not
   paste it into chat, commit it, or put it in a screenshot.
4. On the DuckDNS page, set the current IP to the VM's public IP (the compose
   updater will keep it current afterwards).
5. From Windows, check propagation:

```powershell
Resolve-DnsName scout-example.duckdns.org
```

The answer must be the VM's public IPv4. Caddy cannot issue a certificate until
this is true.

> Using your own domain instead? Point an A record at the VM, set
> `SITE_ADDRESS` to that name, and omit `--with-duckdns` when deploying.

---

## 9. Configure secrets in `deploy/.env`

On the VM, in the repository directory:

```bash
cp deploy/.env.example deploy/.env
chmod 600 deploy/.env
```

Generate the Basic Auth hash (the plaintext password is never stored):

```bash
docker run --rm caddy:2.8-alpine caddy hash-password --plaintext 'choose-a-long-random-password'
```

Use a long, random, unique password (a password manager passphrase is ideal).
The whole app is protected by this one credential; anyone with it can edit and
delete data.

Edit `deploy/.env`:

```bash
nano deploy/.env
```

```dotenv
SITE_ADDRESS=scout-example.duckdns.org
ACME_EMAIL=your-real-email@example.com
BASIC_AUTH_USER=scout
BASIC_AUTH_HASH='$2a$14$...the hash you just generated...'
DUCKDNS_SUBDOMAIN=scout-example
DUCKDNS_TOKEN=...your DuckDNS token...
TZ=UTC
BACKUP_DIR=/var/backups/discord-community-scout
BACKUP_RETENTION_DAYS=14
```

**The hash must be inside single quotes.** bcrypt hashes contain `$`, and
Docker Compose would otherwise try to expand `$...` sequences and corrupt the
hash. `scripts/deploy.sh` validates the hash format and fails with that hint if
it is wrong.

Verify the secret files are ignored before you ever commit:

```bash
git status --short          # deploy/.env must not appear
git check-ignore -v deploy/.env
```

---

## 10. Deploy

```bash
./scripts/deploy.sh --with-duckdns
```

(Use `./scripts/deploy.sh` without the flag for a non-DuckDNS domain.)

The script:

1. loads and validates `deploy/.env` (hostname, email, bcrypt hash format,
   DuckDNS name match);
2. runs `docker compose config` and `caddy validate` before starting anything;
3. builds the image for the VM's native architecture (first build takes several
   minutes on the A1 - the Rust dependency graph is compiled once and cached);
4. starts app + Caddy (+ DuckDNS updater), waits for the app healthcheck;
5. prints the running services and the expected URL.

Watch certificate issuance:

```bash
docker compose --env-file deploy/.env -f deploy/docker-compose.yml logs -f caddy
```

Expect "certificate obtained successfully" within a minute or two of DNS
propagation. If it fails, see section 15.

---

## 11. Verify the live deployment

From the VM (or any machine with bash + curl):

```bash
SCOUT_PASSWORD='your-password' ./scripts/verify-deployment.sh \
  https://scout-example.duckdns.org scout --full
```

It checks, and fails non-zero on any problem:

- unauthenticated `GET /` and `GET /api/health` return **401**;
- `http://` redirects to `https://`;
- the TLS certificate validates (no `-k`);
- authenticated `/api/health` returns `200` with `"database":"ok"`;
- the frontend HTML loads;
- the app container publishes **no** host port;
- with `--full`: create + edit a community, generate a draft and assert the
  remote/commission/$12/recovery disclosures, preview + commit + export CSV,
  then delete both temporary records.

Manual checks afterwards:

1. Open `https://scout-example.duckdns.org` in Chrome on desktop and on a phone.
2. The browser prompts for the username and password (Basic Auth).
3. Add a community by hand, set its promotion policy, and confirm it survives a
   page reload.
4. Import a small CSV, export it back, and compare.
5. Generate a draft; confirm it discloses remote work, commission-based pay, the
   $12 deposit and its conditional recovery. Copy it - the app never posts.
6. Restart and confirm persistence:

```bash
docker compose --env-file deploy/.env -f deploy/docker-compose.yml restart
```

Then reload the page and confirm your data is still there.

7. Reboot test (optional, but proves startup order):

```bash
sudo reboot
```

After reconnecting, `docker compose ps` should show all services healthy without
manual intervention.

---

## 12. Backups, restore, and VM-loss recovery

### What is protected

- The SQLite database is **not** in Git and **not** exposed by the web server.
- Backups are compressed copies on the VM, integrity-checked before they are
  accepted.
- **Until you configure the optional off-site copy, backups protect you against
  corruption or accidental deletion, not against losing the VM.** The daily
  timer is not a substitute for off-site storage.

### Install the daily backup timer

```bash
sudo ./scripts/install-backup-timer.sh            # daily at 03:15 UTC
sudo systemctl start scout-backup.service         # run one now
journalctl -u scout-backup -n 40 --no-pager
```

Backups go to `/var/backups/discord-community-scout` by default, named
`scout-YYYYMMDD-HHMMSS.db.gz`, with a `.sha256` next to each file. Files older
than `BACKUP_RETENTION_DAYS` (default 14) are pruned.

### Verify a backup

```bash
sudo ./scripts/verify-backup.sh /var/backups/discord-community-scout/scout-*.db.gz
```

It checks the checksum (when present), `PRAGMA integrity_check`,
`PRAGMA foreign_key_check`, the expected tables and row counts.

### Required restore test

Do this once, before claiming backups work:

1. Create a backup: `sudo ./scripts/backup-db.sh`.
2. In the web UI, note the number of communities.
3. Add a temporary community (e.g. "restore-test").
4. Restore the backup:

```bash
sudo ./scripts/restore-db.sh --latest --yes
```

5. Confirm: the temporary community is gone, the original records are present,
   `/api/health` is ok, and the UI loads.
6. The pre-restore database is kept inside the volume as
   `scout.db.pre-restore-<timestamp>`. Delete it once you are satisfied:

```bash
docker run --rm -v discord-community-scout_app_data:/data alpine \
  sh -c 'rm -f /data/scout.db.pre-restore-*'
```

Record the date and result of this test. Do not describe backups as working
before it passes.

If a restore fails, `restore-db.sh` automatically puts the previous database
back and restarts the app.

### Optional: encrypted off-site backups (OCI Object Storage + age + rclone)

Always Free Object Storage can hold encrypted backups so a lost VM does not
mean lost data. This is **not configured by default and has not been tested in
this repository's environment**. Set it up deliberately:

1. In the Oracle console, create a bucket in your home region (Object Storage,
   Always Free tier). Note the namespace and region.
2. Create a **Customer Secret Key** for your user (S3-compatible access).
3. On the VM, install and configure rclone (enter the key/secret interactively;
   never put them in the repo):

```bash
sudo apt-get install -y rclone age
rclone config          # new remote -> s3 -> Other provider -> endpoint
                       # https://<namespace>.compat.objectstorage.<region>.oraclecloud.com
rclone ls <remote>:<bucket>   # must succeed before continuing
```

4. Create an age key pair **on your Windows PC** (keep the private key off the
   VM and back it up somewhere safe):

```powershell
# after installing age (e.g. winget install FiloSottile.age)
age-keygen -o "$env:USERPROFILE\scout-backup-key.txt"
Get-Content "$env:USERPROFILE\scout-backup-key.txt"
```

Put the **public** key (`age1...`) into `deploy/.env`:

```dotenv
BACKUP_AGE_RECIPIENT=age1...
BACKUP_RCLONE_REMOTE=<remote>:<bucket>
```

5. Run a backup and verify the remote copy end to end. On the VM:

```bash
sudo ./scripts/backup-db.sh
rclone ls <remote>:<bucket>
```

Copy one encrypted file to your PC and decrypt it there (PowerShell, with `age`
installed):

```powershell
rclone copy "<remote>:<bucket>/scout-....db.gz.age" .
age -d -i "$env:USERPROFILE\scout-backup-key.txt" -o scout-backup.db.gz "scout-....db.gz.age"
```

Then verify the decrypted backup from WSL/Git Bash in the repository:

```bash
./scripts/verify-backup.sh /mnt/c/Users/<you>/scout-backup.db.gz
```

On Windows, decompress with 7-Zip if needed and check the plain `.db` with
`sqlite3`'s `PRAGMA integrity_check` if you do not have WSL/Git Bash.

Only after a downloaded, decrypted copy passes verification may you state that
off-site backups work. Keep the retention/pruning in mind: the script prunes
local files only.

### Recovery if the VM is lost or reclaimed

1. Create a new Always Free VM **with the same steps in section 5** (new SSH
   key is fine; reuse the old one if you still have it).
2. Run `scripts/install-vm.sh` and clone the repository.
3. Recreate `deploy/.env` (hostname, email, a **new** bcrypt hash, DuckDNS token
   if applicable).
4. Get the latest backup onto the new VM. With off-site storage:
   `rclone copy <remote>:<bucket>/scout-....db.gz.age ./` then decrypt with age.
   Without off-site storage, the data is gone with the VM - this is the risk the
   optional step exists to remove.
5. Start the stack once (`./scripts/deploy.sh --with-duckdns`) so the volume and
   database exist, then restore instead of starting empty:
   `sudo ./scripts/restore-db.sh ./scout-....db.gz --yes`
6. Point DuckDNS at the new public IP (the updater does this automatically once
   it starts; otherwise update the IP on the DuckDNS site).
7. Wait for Caddy to issue a new certificate and rerun
   `scripts/verify-deployment.sh ... --full`.
8. Reinstall the backup timer.

Record the recovery time and the backup used. A recovery that has never been
exercised is a plan, not a guarantee.

---

## 13. Operations

```bash
cd ~/discord-community-scout

# status and health
docker compose --env-file deploy/.env -f deploy/docker-compose.yml ps

# logs
docker compose --env-file deploy/.env -f deploy/docker-compose.yml logs --tail=100 app
docker compose --env-file deploy/.env -f deploy/docker-compose.yml logs --tail=100 caddy

# update to the latest main
git pull --ff-only
./scripts/deploy.sh --no-build   # or drop --no-build to rebuild

# stop / start
docker compose --env-file deploy/.env -f deploy/docker-compose.yml down
./scripts/deploy.sh
```

- Certificates renew automatically; `caddy_data` must be kept.
- Security updates are installed automatically (`unattended-upgrades`).
- Rebuild after a `Cargo.toml`/source change; `--no-build` reuses the image.
- The app healthcheck hits `/api/health` inside the container, so it is not
  affected by Basic Auth.
- Docker Compose must always be given `--env-file deploy/.env -f
  deploy/docker-compose.yml` (the helper scripts do this), otherwise the
  required variables are missing.

---

## 14. Security model and limitations

Implemented:

- HTTPS only for authenticated use; HTTP redirects to HTTPS; HSTS.
- **All** routes (frontend, API, CSV export) require Basic Auth at the proxy.
- Only a bcrypt hash is stored in Caddy's environment; the plaintext password is
  never written to the repo or the VM disk by these scripts.
- The app port is unpublished and reachable only on the internal Docker network.
- Oracle cloud firewall + `ufw` allow only 22/80/443.
- Secrets (`deploy/.env`) are git-ignored and checked before commit.
- Caddy admin API disabled; security headers set; `X-Robots-Tag: noindex`.
- Backup files are integrity-checked and can be encrypted off-site.

Limitations to understand:

- **One shared credential.** There are no per-user accounts or audit trails; a
  person with the password has full access. Use a long random password.
- Caddy core has **no rate limiting or account lockout** on Basic Auth. Do not
  publish the hostname widely; prefer a long password; the server still
  enforces auth on every request.
- The app has no CSRF tokens because the API is JSON-only behind Basic Auth and
  the UI sends `Content-Type: application/json`; Basic Auth credentials are
  attached by the browser to same-origin requests. This is acceptable for a
  single-operator tool but is another reason to keep the URL private.
- SQLite is single-writer and local. Run **one** app container; do not put the
  volume on NFS or scale horizontally.
- The opportunity/company is never verified by the app; drafts require human
  review and manual posting. The app never posts, joins, or DMs.
- No uptime monitoring is included (paid monitoring is out of scope). Check the
  site periodically.

---

## 15. Troubleshooting

| Symptom | Likely cause / action |
| --- | --- |
| `deploy.sh` stops at "BASIC_AUTH_HASH is not a valid bcrypt hash" | hash was not single-quoted in `deploy/.env`, or the wrong value was pasted. Regenerate and wrap in `'...'`. |
| Caddy logs `no such host` / certificate pending | DNS not pointing at the VM. `Resolve-DnsName <host>` on Windows; fix DuckDNS; wait and retry. |
| Certificate challenge fails | Port 80 blocked by the Oracle Security List or host firewall. Check both; `curl -I http://<host>` should reach Caddy. |
| `502 Bad Gateway` | App container unhealthy. Check `docker compose ... logs app` and `/api/health` inside the container. |
| Browser keeps asking for the password | Wrong password, or the hash/hostname changed. Regenerate the hash and redeploy. |
| Site unreachable but containers are healthy | Oracle Security List missing 80/443, or host firewall/SELinux/iptables conflict. Check `sudo ufw status` and the cloud rules. |
| "Out of host capacity" when creating the VM | A1 capacity is unavailable. Retry later, try another availability domain, reduce to 2 OCPU/12 GB. Do not choose a paid shape. |
| VM stopped/terminated unexpectedly | Possibly the idle-instance policy. Rebuild and restore from backup (section 12). Do not add artificial load. |
| `docker compose` complains variables are not set | You ran it without `--env-file deploy/.env -f deploy/docker-compose.yml`. Use the scripts. |
| Restore says integrity check failed | Do not proceed. Try another backup; keep the current database. Verify the file with `scripts/verify-backup.sh`. |

---

## 16. Completion checklist

- [ ] Oracle account created, home region Hyderabad/Mumbai (or nearest available)
- [ ] Always Free A1 VM running with public IPv4, no paid resources
- [ ] SSH key uploaded; console Security List allows 22 (restricted), 80, 443
- [ ] `install-vm.sh` completed; `docker compose version` works
- [ ] DuckDNS hostname resolves to the VM (or own domain A record)
- [ ] `deploy/.env` present, `chmod 600`, hash single-quoted, token not committed
- [ ] `deploy.sh` completes and all containers are healthy
- [ ] `verify-deployment.sh --full` passes without `-k`
- [ ] Desktop and mobile browsers load the app behind Basic Auth
- [ ] Create/edit/CSV/draft workflows tested on the live site
- [ ] Data survives `docker compose restart` and a VM reboot
- [ ] Daily backup timer installed and one backup verified
- [ ] A **restore** has actually succeeded
- [ ] (Recommended) encrypted off-site copy configured **and** test-restored
- [ ] README/ROADMAP updated with the real URL and date
