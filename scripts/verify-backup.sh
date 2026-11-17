#!/usr/bin/env bash
# Verify a SQLite backup file: checksum (when present), integrity, schema and
# row counts. Exits non-zero if anything is wrong.
#
#   ./scripts/verify-backup.sh /var/backups/discord-community-scout/scout-20260101-030000.db.gz
#
# Encrypted (.age) files must be decrypted first; this script does not hold
# your private key.

set -euo pipefail

# shellcheck source=lib.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

[ $# -eq 1 ] || {
	echo "usage: $0 <backup.db|backup.db.gz>" >&2
	exit 2
}
FILE="$1"
[ -f "$FILE" ] || die "no such file: $FILE"

command -v sqlite3 >/dev/null 2>&1 || die "sqlite3 is not installed"

case "$FILE" in
*.age) die "this backup is encrypted; decrypt it first, e.g. age -d -i key.txt -o backup.db $FILE" ;;
esac

if [ -f "$FILE.sha256" ]; then
	echo "==> Checking sha256"
	(cd "$(dirname "$FILE")" && sha256sum -c "$(basename "$FILE.sha256")")
else
	echo "==> No .sha256 file next to the backup; skipping checksum"
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

DB="$FILE"
case "$FILE" in
*.gz)
	echo "==> Decompressing"
	gzip -dc "$FILE" >"$WORK/backup.db"
	DB="$WORK/backup.db"
	;;
esac

echo "==> PRAGMA integrity_check"
INTEGRITY="$(sqlite3 "$DB" 'PRAGMA integrity_check;')"
[ "$INTEGRITY" = "ok" ] || die "integrity_check failed: $INTEGRITY"
echo "    ok"

echo "==> PRAGMA foreign_key_check"
FK="$(sqlite3 "$DB" 'PRAGMA foreign_key_check;')"
[ -z "$FK" ] || die "foreign_key_check reported rows: $FK"
echo "    ok"

echo "==> Schema"
TABLES="$(sqlite3 "$DB" "SELECT name FROM sqlite_master WHERE type='table' AND name IN ('communities','community_sources','discovery_runs','settings') ORDER BY name;")"
EXPECTED=$'communities\ncommunity_sources\ndiscovery_runs\nsettings'
[ "$TABLES" = "$EXPECTED" ] || die "missing expected tables; found: $(printf '%s' "$TABLES" | tr '\n' ' ')"
echo "    communities, community_sources, discovery_runs, settings present"

echo "==> Row counts"
COMMUNITIES="$(sqlite3 "$DB" 'SELECT COUNT(*) FROM communities;')"
SOURCES="$(sqlite3 "$DB" 'SELECT COUNT(*) FROM community_sources;')"
RUNS="$(sqlite3 "$DB" 'SELECT COUNT(*) FROM discovery_runs;')"
echo "    communities=$COMMUNITIES community_sources=$SOURCES discovery_runs=$RUNS"

echo "==> Sample row check"
sqlite3 "$DB" "SELECT id, name FROM communities ORDER BY updated_at DESC LIMIT 3;" || true

echo
echo "PASS: $FILE is a readable, consistent backup"
