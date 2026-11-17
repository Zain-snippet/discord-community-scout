#!/usr/bin/env bash
# Install a systemd timer that runs scripts/backup-db.sh every day.
#
#   sudo ./scripts/install-backup-timer.sh [--at 03:15]
#
# Logs go to the journal: journalctl -u scout-backup -n 50
# Remove with: systemctl disable --now scout-backup.timer && rm /etc/systemd/system/scout-backup.*
#
# This only schedules backups on the VM. Off-site/encrypted copies are a
# separate, optional step (see docs/deployment.md).

set -euo pipefail

# shellcheck source=lib.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

require_root

AT="03:15"
while [ $# -gt 0 ]; do
	case "$1" in
	--at)
		AT="${2:?--at needs a time like 03:15}"
		shift 2
		;;
	-h | --help)
		sed -n '2,12p' "$0"
		exit 0
		;;
	*)
		echo "unknown argument: $1" >&2
		exit 2
		;;
	esac
done

case "$AT" in
[0-2][0-9]:[0-5][0-9]) ;;
*) die "--at must look like HH:MM (24h), got $AT" ;;
esac

BACKUP_SCRIPT="$REPO_ROOT/scripts/backup-db.sh"
[ -x "$BACKUP_SCRIPT" ] || chmod +x "$BACKUP_SCRIPT"

echo "==> Writing /etc/systemd/system/scout-backup.service"
cat >/etc/systemd/system/scout-backup.service <<EOF
[Unit]
Description=Discord Community Scout SQLite backup
After=docker.service
Requires=docker.service

[Service]
Type=oneshot
ExecStart=$BACKUP_SCRIPT
Nice=10
IOSchedulingClass=best-effort
IOSchedulingPriority=7
EOF

echo "==> Writing /etc/systemd/system/scout-backup.timer"
cat >/etc/systemd/system/scout-backup.timer <<EOF
[Unit]
Description=Daily Discord Community Scout backup

[Timer]
OnCalendar=*-*-* $AT:00
Persistent=true
RandomizedDelaySec=10m

[Install]
WantedBy=timers.target
EOF

systemctl daemon-reload
systemctl enable --now scout-backup.timer

echo
systemctl list-timers scout-backup.timer --no-pager || true
echo
echo "Installed. Recorded backups are pruned after BACKUP_RETENTION_DAYS (deploy/.env)."
echo "Test it now with: sudo systemctl start scout-backup.service && journalctl -u scout-backup -n 30 --no-pager"
