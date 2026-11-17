#!/usr/bin/env bash
# Shared helpers for the deployment scripts. Source it; do not execute it.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
COMPOSE_FILE="$REPO_ROOT/deploy/docker-compose.yml"
ENV_FILE="$REPO_ROOT/deploy/.env"

die() {
	echo "error: $*" >&2
	exit 1
}

require_root() {
	[ "$(id -u)" -eq 0 ] || die "run as root, e.g. sudo $0"
}

# Load KEY=VALUE pairs from an env file without executing it as shell code.
load_env() {
	local file="${1:-$ENV_FILE}" line key value
	[ -f "$file" ] || die "missing $file (copy deploy/.env.example to deploy/.env)"
	while IFS= read -r line || [ -n "$line" ]; do
		line="${line%$'\r'}"
		case "$line" in '' | '#'*) continue ;; esac
		case "$line" in *=*) ;; *) continue ;; esac
		key="${line%%=*}"
		value="${line#*=}"
		key="$(printf '%s' "$key" | tr -d '[:space:]')"
		case "$key" in '' | *[!A-Za-z0-9_]*) continue ;; esac
		case "$value" in
		\'*\')
			value="${value#\'}"
			value="${value%\'}"
			;;
		\"*\")
			value="${value#\"}"
			value="${value%\"}"
			;;
		esac
		export "$key=$value"
	done <"$file"
}

compose() {
	docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" "$@"
}

# Host path of the app's named volume (project name is pinned in compose).
data_volume_mountpoint() {
	local volume
	volume="$(docker volume ls -q | grep -E '^discord-community-scout_app_data$' || true)"
	[ -n "$volume" ] || die "volume discord-community-scout_app_data not found; start the stack with scripts/deploy.sh first"
	docker volume inspect -f '{{ .Mountpoint }}' "$volume"
}

app_container_id() {
	compose ps -q app
}

wait_for_app_healthy() {
	local timeout="${1:-240}" elapsed=0 cid status
	cid="$(app_container_id)"
	[ -n "$cid" ] || die "app container is not running"
	while [ "$elapsed" -lt "$timeout" ]; do
		status="$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "$cid" 2>/dev/null || echo unknown)"
		case "$status" in
		healthy) return 0 ;;
		unhealthy | exited | dead)
			docker logs --tail 50 "$cid" >&2 || true
			die "app container is $status"
			;;
		esac
		sleep 3
		elapsed=$((elapsed + 3))
	done
	docker logs --tail 50 "$cid" >&2 || true
	die "timed out after ${timeout}s waiting for the app to become healthy"
}
