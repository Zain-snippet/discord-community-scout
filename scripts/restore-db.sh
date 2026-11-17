#!/usr/bin/env bash
# Restore a SQLite backup into the running stack's data volume.
#
#   sudo ./scripts/restore-db.sh <backup.db|backup.db.gz> [--yes]
#   sudo ./scripts/restore-db.sh --latest [--yes]
#
# The current database is kept as scout.db.pre-restore-<timestamp> inside the
# volume. The app is stopped during the swap and started again afterwards.
# Run scripts/verify-backup.sh first if you want to inspect the file.

set -euo pipefail

# shellcheck source=lib.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

require_root
load_env

ASSUME_YES=0
FILE=""

while [ $# -gt 0 ]; do
	case "$1" in
	--latest)
		FILE="__latest__"
		shift
		;;
	--yes | -y)
		ASSUME_YES=1
		shift
		;;
	-h | --help)
		sed -n '2,10p' "$0"
		exit 0
		;;
	-*)
		echo "unknown option: $1 (try --help)" >&2
		exit 2
		;;
	*)
		[ -z "$FILE" ] || die "only one backup file may be given"
		FILE="$1"
		shift
		;;
	esac
done

[ -n "$FILE" ] || {
	echo "usage: $0 <backup.db|backup.db.gz>|--latest [--yes]" >&2
	exit 2
}

if [ "$FILE" = "__latest__" ]; then
	FILE="$(find "${BACKUP_DIR:-/var/backups/discord-community-scout}" -maxdepth 1 -type f -name 'scout-*.db.gz' -printf '%T@ %p\n' 2>/dev/null |
		sort -n | tail -1 | cut -d' ' -f2-)"
	[ -n "$FILE" ] || die "no backups found in ${BACKUP_DIR:-/var/backups/discord-community-scout}"
	echo "==> Using latest backup: $FILE"
fi

[ -f "$FILE" ] || die "no such file: $FILE"
case "$FILE" in
*.age) die "this backup is encrypted; decrypt it first, e.g. age -d -i key.txt -o backup.db $FILE" ;;
esac

echo "==> Verifying the backup before restoring"
"$SCRIPT_DIR/verify-backup.sh" "$FILE"

MOUNT="$(data_volume_mountpoint)"
DB="$MOUNT/scout.db"
STAMP="$(date -u +%Y%m%d-%H%M%S)"
PREV="$MOUNT/scout.db.pre-restore-$STAMP"

if [ "$ASSUME_YES" -ne 1 ]; then
	echo
	echo "This will REPLACE the live database in $MOUNT with $FILE."
	read -r -p "Type 'restore' to continue: " answer
	[ "$answer" = "restore" ] || die "aborted"
fi

APP_CID="$(app_container_id || true)"
APP_WAS_RUNNING=0
if [ -n "$APP_CID" ]; then
	APP_WAS_RUNNING=1
	echo "==> Stopping the app container"
	compose stop app
fi

RESTORE_OK=0
cleanup() {
	if [ "$RESTORE_OK" -eq 1 ]; then
		return
	fi
	echo "restore did not complete; rolling back to the previous database" >&2
	if [ -f "$PREV" ]; then
		rm -f "$MOUNT/scout.db" "$MOUNT/scout.db-wal" "$MOUNT/scout.db-shm"
		cp -a "$PREV" "$MOUNT/scout.db"
	fi
	if [ "$APP_WAS_RUNNING" -eq 1 ]; then
		compose start app >/dev/null 2>&1 || true
	fi
}
trap cleanup EXIT

echo "==> Keeping the current database as $(basename "$PREV")"
if [ -f "$DB" ]; then
	cp -a "$DB" "$PREV"
fi

WORK="$(mktemp -d)"
SOURCE="$FILE"
case "$FILE" in
*.gz)
	gzip -dc "$FILE" >"$WORK/backup.db"
	SOURCE="$WORK/backup.db"
	;;
esac

echo "==> Installing the backup"
cp "$SOURCE" "$MOUNT/.scout-restore.tmp"
rm -f "$MOUNT/scout.db-wal" "$MOUNT/scout.db-shm"
mv -f "$MOUNT/.scout-restore.tmp" "$DB"
chown 0:0 "$DB"
chmod 600 "$DB"
rm -rf "$WORK"

echo "==> Checking the restored database"
INTEGRITY="$(sqlite3 "$DB" 'PRAGMA integrity_check;')"
[ "$INTEGRITY" = "ok" ] || die "restored database failed integrity_check: $INTEGRITY"
ROWS="$(sqlite3 "$DB" 'SELECT COUNT(*) FROM communities;')"
echo "    integrity ok, $ROWS communities"

if [ "$APP_WAS_RUNNING" -eq 1 ]; then
	echo "==> Starting the app"
	compose start app
	wait_for_app_healthy 300
	HEALTH="$(compose exec -T app curl -fsS http://127.0.0.1:8080/api/health || true)"
	echo "    health: $HEALTH"
else
	echo "==> The app was not running; start it with scripts/deploy.sh"
fi

RESTORE_OK=1
trap - EXIT

echo
echo "Restore complete. Previous database kept at: $PREV"
