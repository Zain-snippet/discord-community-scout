#!/usr/bin/env bash
# Check a deployed instance from the outside: HTTPS, redirect, Basic Auth,
# health, frontend, and (with --full) create/edit/draft/CSV/export/delete.
#
#   SCOUT_PASSWORD='...' ./scripts/verify-deployment.sh https://scout.example.duckdns.org scout
#   SCOUT_PASSWORD='...' ./scripts/verify-deployment.sh https://... scout --full
#
# Run this on the VM (or any machine with curl). It needs the Basic Auth
# password, which is never written to disk by this script.

set -uo pipefail

BASE="${1:-}"
USER_NAME="${2:-}"
MODE="${3:-}"
[ -n "$BASE" ] && [ -n "$USER_NAME" ] || {
	echo "usage: SCOUT_PASSWORD='...' $0 <https-url> <username> [--full]" >&2
	exit 2
}
case "$MODE" in '' | --full) ;; *)
	echo "unknown option: $MODE (only --full is supported)" >&2
	exit 2
	;; esac

command -v curl >/dev/null 2>&1 || {
	echo "curl is required" >&2
	exit 1
}

BASE="${BASE%/}"
case "$BASE" in
https://*) ;;
*)
	echo "the URL must start with https:// (found: $BASE)" >&2
	exit 2
	;;
esac

PASSWORD="${SCOUT_PASSWORD:-}"
if [ -z "$PASSWORD" ]; then
	if [ -t 0 ]; then
		read -r -s -p "Basic Auth password for ${USER_NAME}: " PASSWORD
		echo
	else
		echo "set SCOUT_PASSWORD in the environment when running non-interactively" >&2
		exit 2
	fi
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

FAILURES=0
pass() { echo "PASS  $*"; }
fail() {
	echo "FAIL  $*" >&2
	FAILURES=$((FAILURES + 1))
}

# request METHOD URL CURL_ARGS... -> writes body/headers to $TMP, echoes status
request() {
	local method="$1" url="$2"
	shift 2
	curl -sS --max-time 30 -o "$TMP/body" -D "$TMP/headers" -w '%{http_code}' \
		-X "$method" "$@" "$url" 2>"$TMP/curl.err" || true
}

body() { cat "$TMP/body"; }
has_header() { grep -qiE "^$1:" "$TMP/headers"; }

echo "==> Testing $BASE"
echo

# 1. Unauthenticated access is refused, for the frontend and the API.
STATUS="$(request GET "$BASE/")"
if [ "$STATUS" = "401" ] && has_header 'www-authenticate'; then
	pass "unauthenticated GET / -> 401 with WWW-Authenticate"
else
	fail "unauthenticated GET / expected 401 with WWW-Authenticate, got $STATUS"
fi

STATUS="$(request GET "$BASE/api/health")"
if [ "$STATUS" = "401" ]; then
	pass "unauthenticated GET /api/health -> 401 (API is protected too)"
else
	fail "unauthenticated GET /api/health expected 401, got $STATUS"
fi

# 2. HTTP is redirected to HTTPS.
HTTP_BASE="http://${BASE#https://}"
STATUS="$(request GET "$HTTP_BASE/")"
LOCATION="$(grep -i '^location:' "$TMP/headers" | head -1 | tr -d '\r' | cut -d' ' -f2-)"
case "$STATUS" in
301 | 302 | 307 | 308)
	if [ "${LOCATION#https://}" != "$LOCATION" ]; then
		pass "HTTP -> HTTPS redirect ($STATUS to $LOCATION)"
	else
		fail "HTTP redirect does not target HTTPS: $LOCATION"
	fi
	;;
*)
	fail "expected an HTTP->HTTPS redirect, got $STATUS"
	;;
esac

# 3. Certificate is valid (curl verifies by default).
SSL_RESULT="$(curl -sS -o /dev/null -w '%{ssl_verify_result}' --max-time 30 \
	-u "${USER_NAME}:${PASSWORD}" "$BASE/" 2>/dev/null || echo "error")"
if [ "$SSL_RESULT" = "0" ]; then
	pass "TLS certificate validates"
else
	fail "TLS verification failed (ssl_verify_result=$SSL_RESULT)"
fi

# 4. Authenticated health endpoint.
STATUS="$(request GET "$BASE/api/health" -u "${USER_NAME}:${PASSWORD}")"
if [ "$STATUS" = "200" ] && grep -q '"database":"ok"' "$TMP/body" && grep -q '"status":"ok"' "$TMP/body"; then
	pass "authenticated /api/health -> 200, database ok"
else
	fail "authenticated /api/health expected 200 database ok, got $STATUS: $(body | head -c 200)"
fi

# 5. Frontend loads.
STATUS="$(request GET "$BASE/" -u "${USER_NAME}:${PASSWORD}")"
if [ "$STATUS" = "200" ] && grep -qi '<title>Discord Community Scout</title>' "$TMP/body"; then
	pass "frontend HTML loads"
else
	fail "frontend expected the app title, got $STATUS"
fi

# 6. The app port is not published on the host (only run where docker exists).
if command -v docker >/dev/null 2>&1; then
	PORTS="$(docker ps --format '{{.Names}} {{.Ports}}' 2>/dev/null | grep 'discord-community-scout-app' || true)"
	if [ -z "$PORTS" ]; then
		echo "SKIP  no local app container found for the port check"
	elif printf '%s' "$PORTS" | grep -q '8080->'; then
		fail "the app container publishes port 8080: $PORTS"
	else
		pass "app container does not publish its internal port"
	fi
else
	echo "SKIP  docker not installed; port check skipped"
fi

if [ "$MODE" = "--full" ]; then
	echo
	echo "==> Full workflow tests (temporary records are deleted afterwards)"
	STAMP="$(date -u +%Y%m%d%H%M%S)"
	CREATED_ID=""
	CSV_ID=""

	NAME="Deployment smoke test $STAMP"
	STATUS="$(request POST "$BASE/api/communities" \
		-u "${USER_NAME}:${PASSWORD}" -H 'Content-Type: application/json' \
		--data "{\"name\":\"$NAME\",\"description\":\"temporary record from verify-deployment.sh\",\"tags\":[\"testing\"],\"promotion_policy\":\"unknown\"}")"
	CREATED_ID="$(body | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4)"
	if [ "$STATUS" = "201" ] && [ -n "$CREATED_ID" ]; then
		pass "create community -> 201 ($CREATED_ID)"
	else
		fail "create community expected 201, got $STATUS: $(body | head -c 200)"
	fi

	if [ -n "$CREATED_ID" ]; then
		EDITED="$NAME (edited)"
		STATUS="$(request PATCH "$BASE/api/communities/$CREATED_ID" \
			-u "${USER_NAME}:${PASSWORD}" -H 'Content-Type: application/json' \
			--data "{\"name\":\"$EDITED\",\"description\":\"edited by verify-deployment.sh\",\"tags\":[\"testing\"]}")"
		if [ "$STATUS" = "200" ] && grep -q 'edited by verify-deployment.sh' "$TMP/body"; then
			pass "edit community -> 200"
		else
			fail "edit community expected 200, got $STATUS"
		fi

		STATUS="$(request POST "$BASE/api/message/draft" \
			-u "${USER_NAME}:${PASSWORD}" -H 'Content-Type: application/json' \
			--data "{\"community_id\":\"$CREATED_ID\",\"tone\":\"conversational\",\"variant\":\"concise\"}")"
		DRAFT="$(body | tr 'A-Z' 'a-z')"
		if [ "$STATUS" = "200" ] &&
			grep -q 'remote' <<<"$DRAFT" &&
			grep -q 'commission' <<<"$DRAFT" &&
			grep -q '\$12' <<<"$DRAFT" &&
			grep -q 'recoverable after your first payment' <<<"$DRAFT"; then
			pass "message draft includes remote, commission, \$12 and recovery disclosures"
		else
			fail "message draft missing a mandatory disclosure (status $STATUS)"
		fi
	fi

	CSV_NAME="CSV smoke test $STAMP"
	CSV_TEXT="name,description,invite_url,canonical_url,tags,language,channel_name,user_notes,promotion_policy,policy_evidence,review_status,member_count\n${CSV_NAME},temporary CSV row,,,,,,,,,,\n"
	STATUS="$(request POST "$BASE/api/csv/preview" \
		-u "${USER_NAME}:${PASSWORD}" -H 'Content-Type: application/json' \
		--data "{\"csv\":\"${CSV_TEXT}\"}")"
	if [ "$STATUS" = "200" ] && grep -q 'CSV smoke test' "$TMP/body"; then
		pass "CSV import preview parses a new row"
	else
		fail "CSV preview expected 200 with the row, got $STATUS"
	fi

	STATUS="$(request POST "$BASE/api/csv/commit" \
		-u "${USER_NAME}:${PASSWORD}" -H 'Content-Type: application/json' \
		--data "{\"csv\":\"${CSV_TEXT}\"}")"
	if [ "$STATUS" = "200" ] && grep -q '"imported":1' "$TMP/body"; then
		pass "CSV commit imports the row"
	else
		fail "CSV commit expected imported=1, got $STATUS: $(body | head -c 200)"
	fi

	STATUS="$(request GET "$BASE/api/communities?per_page=100&q=CSV%20smoke%20test%20$STAMP" -u "${USER_NAME}:${PASSWORD}")"
	CSV_ID="$(body | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4)"

	STATUS="$(request GET "$BASE/api/csv/export" -u "${USER_NAME}:${PASSWORD}")"
	if [ "$STATUS" = "200" ] && grep -q 'CSV smoke test' "$TMP/body"; then
		pass "CSV export includes the imported row"
	else
		fail "CSV export expected the imported row, got $STATUS"
	fi

	# Cleanup: never leave smoke-test records behind.
	for id in "$CREATED_ID" "$CSV_ID"; do
		[ -n "$id" ] || continue
		SDEL="$(request DELETE "$BASE/api/communities/$id?confirm=true" -u "${USER_NAME}:${PASSWORD}")"
		if [ "$SDEL" = "204" ]; then
			pass "deleted temporary community $id"
		else
			fail "could not delete temporary community $id (status $SDEL)"
		fi
	done
fi

echo
if [ "$FAILURES" -eq 0 ]; then
	echo "ALL CHECKS PASSED for $BASE"
	exit 0
fi
echo "$FAILURES CHECK(S) FAILED for $BASE" >&2
exit 1
