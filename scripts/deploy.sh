#!/usr/bin/env bash
# Build and start Discord Community Scout behind Caddy (HTTPS + Basic Auth).
#
#   ./scripts/deploy.sh                  # app + Caddy
#   ./scripts/deploy.sh --with-duckdns   # also run the DuckDNS updater profile
#   ./scripts/deploy.sh --no-build       # reuse the image already on the VM
#
# Requires Docker Compose v2 and deploy/.env (copied from deploy/.env.example).

set -euo pipefail

# shellcheck source=lib.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

WITH_DUCKDNS=0
BUILD=1

while [ $# -gt 0 ]; do
	case "$1" in
	--with-duckdns) WITH_DUCKDNS=1 ;;
	--no-build) BUILD=0 ;;
	-h | --help)
		sed -n '2,9p' "$0"
		exit 0
		;;
	*)
		echo "unknown argument: $1 (try --help)" >&2
		exit 2
		;;
	esac
	shift
done

command -v docker >/dev/null 2>&1 || die "docker is not installed; run sudo ./scripts/install-vm.sh first"
docker compose version >/dev/null 2>&1 || die "docker compose v2 is not available"
[ -f "$COMPOSE_FILE" ] || die "missing $COMPOSE_FILE"

echo "==> Checking deploy/.env"
load_env "$ENV_FILE"

[ -n "${SITE_ADDRESS:-}" ] || die "SITE_ADDRESS is empty in deploy/.env"
[ "$SITE_ADDRESS" != "scout-example.duckdns.org" ] ||
	die "SITE_ADDRESS is still the example value in deploy/.env"
case "$SITE_ADDRESS" in
*"://"* | */* | *" "* | *'	'*) die "SITE_ADDRESS must be a bare hostname (no scheme, path or spaces)" ;;
esac

[ -n "${ACME_EMAIL:-}" ] || die "ACME_EMAIL is empty in deploy/.env"
[ "$ACME_EMAIL" != "you@example.com" ] || die "ACME_EMAIL is still the example value in deploy/.env"
case "$ACME_EMAIL" in *@*.*) ;; *) die "ACME_EMAIL does not look like an email address" ;; esac

[ -n "${BASIC_AUTH_USER:-}" ] || die "BASIC_AUTH_USER is empty in deploy/.env"
[ -n "${BASIC_AUTH_HASH:-}" ] || die "BASIC_AUTH_HASH is empty in deploy/.env"
if ! [[ "$BASIC_AUTH_HASH" =~ ^\$2[aby]?\$[0-9]{2}\$[./A-Za-z0-9]{53}$ ]]; then
	die "BASIC_AUTH_HASH is not a valid bcrypt hash.
   Generate one with:
     docker run --rm caddy:2.8-alpine caddy hash-password --plaintext 'your-password'
   Paste the result into deploy/.env INSIDE SINGLE QUOTES ('\$2a\$...'), otherwise
   docker compose expands the '\$' characters and corrupts the hash."
fi

if [ "$WITH_DUCKDNS" -eq 1 ]; then
	[ -n "${DUCKDNS_SUBDOMAIN:-}" ] || die "--with-duckdns requires DUCKDNS_SUBDOMAIN in deploy/.env"
	[ -n "${DUCKDNS_TOKEN:-}" ] || die "--with-duckdns requires DUCKDNS_TOKEN in deploy/.env"
	case "$SITE_ADDRESS" in
	"${DUCKDNS_SUBDOMAIN}.duckdns.org") ;;
	*) die "SITE_ADDRESS ($SITE_ADDRESS) does not match ${DUCKDNS_SUBDOMAIN}.duckdns.org" ;;
	esac
fi

echo "==> Validating the compose configuration"
compose config --quiet

PROFILE_ARGS=()
if [ "$WITH_DUCKDNS" -eq 1 ]; then
	PROFILE_ARGS+=(--profile duckdns)
fi

# Quick sanity check that Caddy can parse the Caddyfile with these values.
echo "==> Validating the Caddyfile"
compose run --rm --no-deps \
	-e SITE_ADDRESS -e ACME_EMAIL -e BASIC_AUTH_USER -e BASIC_AUTH_HASH \
	--entrypoint caddy caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile

if [ "$BUILD" -eq 1 ]; then
	echo "==> Building the application image (this takes a few minutes on first run)"
	compose build --pull app
else
	echo "==> Skipping build (--no-build)"
fi

echo "==> Starting containers"
compose "${PROFILE_ARGS[@]}" up -d --remove-orphans

echo "==> Waiting for the app to become healthy"
wait_for_app_healthy 300

echo
compose ps
echo
echo "Application is running behind Caddy at https://${SITE_ADDRESS}"
echo "Certificate issuance is logged by Caddy; watch it with:"
echo "  docker compose --env-file deploy/.env -f deploy/docker-compose.yml logs -f caddy"
echo
echo "Verify authentication and workflows with:"
echo "  SCOUT_PASSWORD='your-password' ./scripts/verify-deployment.sh https://${SITE_ADDRESS} ${BASIC_AUTH_USER}"
