#!/usr/bin/env bash
# Create a consistent SQLite backup from the running app's data volume,
# verify its integrity, compress it, and prune old copies.
#
#   sudo ./scripts/backup-db.sh [--backup-dir DIR] [--keep DAYS]
#
# Defaults come from deploy/.env:
#   BACKUP_DIR=/var/backups/discord-community-scout
#   BACKUP_RETENTION_DAYS=14
#
# Optional encrypted off-site copy (see docs/deployment.md):
#   BACKUP_AGE_RECIPIENT  encrypt with age before storing
#   BACKUP_RCLONE_REMOTE  copy the (encrypted) file with rclone
#
# The script fails loudly if the integrity check fails. It never reports a
# backup as good without running PRAGMA integrity_check on the copy.

set -euo pipefail

# shellcheck source=lib.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

require_root
load_env

BACKUP_DIR="${BACKUP_DIR:-/var/backups/discord-community-scout}"
RETENTION_DAYS="${BACKUP_RETENTION_DAYS:-14}"

while [ $# -gt 0 ]; do
	case "$1" in
	--backup-dir)
		BACKUP_DIR="${2:?--backup-dir needs a value}"
		shift 2
		;;
	--keep)
		RETENTION_DAYS="${2:?--keep needs a value}"
		shift 2
		;;
	-h | --help)
		sed -n '2,18p' "$0"
		exit 0
		;;
	*)
		echo "unknown argument: $1 (try --help)" >&2
		exit 2
		;;
	esac
done

command -v sqlite3 >/dev/null 2>&1 || die "sqlite3 is not installed (sudo apt-get install sqlite3)"
mkdir -p "$BACKUP_DIR"
chmod 700 "$BACKUP_DIR"

MOUNT="$(data_volume_mountpoint)"
DB="$MOUNT/scout.db"
[ -f "$DB" ] || die "database not found at $DB; is the app running (scripts/deploy.sh)?"

STAMP="$(date -u +%Y%m%d-%H%M%S)"
TMP="$(mktemp "$BACKUP_DIR/.scout-$STAMP.XXXXXX.db")"
trap 'rm -f "$TMP"' EXIT

echo "==> Copying $DB"
# The SQLite backup API handles a live database (including WAL) consistently.
if ! sqlite3 "$DB" ".timeout 15000" ".backup '$TMP'"; then
	die "sqlite3 .backup failed"
fi

echo "==> Checking integrity of the copy"
INTEGRITY="$(sqlite3 "$TMP" 'PRAGMA integrity_check;')"
if [ "$INTEGRITY" != "ok" ]; then
	die "integrity check failed: $INTEGRITY"
fi

ROWS="$(sqlite3 "$TMP" 'SELECT COUNT(*) FROM communities;')"
[ -n "$ROWS" ] || die "could not count communities in the backup"

echo "==> Compressing"
FINAL="$BACKUP_DIR/scout-$STAMP.db.gz"
gzip -9 -c "$TMP" >"$FINAL"
rm -f "$TMP"
trap - EXIT
(cd "$BACKUP_DIR" && sha256sum "$(basename "$FINAL")") >"$FINAL.sha256"

SIZE="$(du -h "$FINAL" | awk '{print $1}')"
echo "    $FINAL ($SIZE, $ROWS communities, integrity ok)"

if [ -n "${BACKUP_AGE_RECIPIENT:-}" ]; then
	command -v age >/dev/null 2>&1 || die "BACKUP_AGE_RECIPIENT is set but age is not installed"
	echo "==> Encrypting with age"
	age -r "$BACKUP_AGE_RECIPIENT" -o "$FINAL.age" "$FINAL"
	rm -f "$FINAL" "$FINAL.sha256"
	FINAL="$FINAL.age"
	(cd "$BACKUP_DIR" && sha256sum "$(basename "$FINAL")") >"$FINAL.sha256"
	echo "    encrypted: $FINAL"
else
	echo "    note: encryption is not configured (BACKUP_AGE_RECIPIENT unset)"
fi

if [ -n "${BACKUP_RCLONE_REMOTE:-}" ]; then
	command -v rclone >/dev/null 2>&1 || die "BACKUP_RCLONE_REMOTE is set but rclone is not installed"
	echo "==> Copying off-site to $BACKUP_RCLONE_REMOTE"
	rclone copyto "$FINAL" "${BACKUP_RCLONE_REMOTE%/}/$(basename "$FINAL")"
	if [ -f "$FINAL.sha256" ]; then
		rclone copyto "$FINAL.sha256" "${BACKUP_RCLONE_REMOTE%/}/$(basename "$FINAL.sha256")"
	fi
	echo "    off-site copy uploaded (verify it with scripts/verify-backup.sh)"
else
	echo "    note: no off-site copy configured (BACKUP_RCLONE_REMOTE unset)"
fi

echo "==> Pruning backups older than ${RETENTION_DAYS} days"
find "$BACKUP_DIR" -maxdepth 1 -type f \
	\( -name 'scout-*.db.gz' -o -name 'scout-*.db.gz.age' \
	-o -name 'scout-*.db.gz.sha256' -o -name 'scout-*.db.gz.age.sha256' \) \
	-mtime +"$RETENTION_DAYS" -print -delete || true

echo
echo "Backup complete: $FINAL"
echo "Restore with: sudo ./scripts/restore-db.sh $FINAL"
