#!/usr/bin/env bash
# Prepare an Ubuntu VM (Oracle Cloud Always Free, ARM64 or AMD64) to run
# Discord Community Scout.
#
#   sudo ./scripts/install-vm.sh [--ssh-port 22] [--user ubuntu] [--no-firewall]
#
# Installs Docker Engine + Compose plugin, sqlite3 (backup scripts), unattended
# security upgrades, and a host firewall that allows only SSH, HTTP and HTTPS.
# Safe to re-run. It does not start the application; use scripts/deploy.sh.
#
# Oracle's Ubuntu images ship a persistent iptables ruleset that rejects all
# inbound traffic except SSH. ufw takes ownership of the host firewall here, so
# netfilter-persistent is disabled to stop it re-applying the old rules at
# boot. A copy of the original file is kept as /etc/iptables/rules.v4.pre-scout.

set -euo pipefail

SSH_PORT=22
DEPLOY_USER="${SUDO_USER:-ubuntu}"
ENABLE_FIREWALL=1

usage() {
	sed -n '2,16p' "$0"
}

while [ $# -gt 0 ]; do
	case "$1" in
	--ssh-port)
		SSH_PORT="${2:?--ssh-port needs a value}"
		shift 2
		;;
	--user)
		DEPLOY_USER="${2:?--user needs a value}"
		shift 2
		;;
	--no-firewall)
		ENABLE_FIREWALL=0
		shift
		;;
	-h | --help)
		usage
		exit 0
		;;
	*)
		echo "unknown argument: $1 (try --help)" >&2
		exit 2
		;;
	esac
done

if [ "$(id -u)" -ne 0 ]; then
	echo "run as root: sudo $0" >&2
	exit 1
fi

export DEBIAN_FRONTEND=noninteractive

if [ -r /etc/os-release ]; then
	# shellcheck disable=SC1091
	. /etc/os-release
	if [ "${ID:-}" != "ubuntu" ]; then
		echo "warning: this script targets Ubuntu; detected '${ID:-unknown}'" >&2
	fi
fi

ARCH="$(dpkg --print-architecture)"
case "$ARCH" in
arm64 | amd64) ;;
*)
	echo "warning: untested architecture '$ARCH'" >&2
	;;
esac
echo "==> Architecture: $ARCH"

echo "==> Installing base packages"
apt-get update -y
apt-get install -y --no-install-recommends \
	ca-certificates curl gnupg sqlite3 unattended-upgrades

# Automatic security updates (Ubuntu defaults, made explicit).
cat >/etc/apt/apt.conf.d/20auto-upgrades <<'EOF'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
EOF
systemctl enable --now unattended-upgrades >/dev/null 2>&1 || true

if [ "$ENABLE_FIREWALL" -eq 1 ]; then
	echo "==> Configuring the host firewall (ufw)"

	# Keep a copy of Oracle's default rules before ufw takes over.
	if [ -f /etc/iptables/rules.v4 ] && [ ! -f /etc/iptables/rules.v4.pre-scout ]; then
		cp /etc/iptables/rules.v4 /etc/iptables/rules.v4.pre-scout
		echo "    backed up /etc/iptables/rules.v4 -> /etc/iptables/rules.v4.pre-scout"
	fi
	if systemctl list-unit-files 2>/dev/null | grep -q '^netfilter-persistent'; then
		systemctl disable --now netfilter-persistent >/dev/null 2>&1 || true
		echo "    disabled netfilter-persistent (ufw owns the rules now)"
	fi

	apt-get install -y --no-install-recommends ufw
	ufw --force default deny incoming
	ufw --force default allow outgoing
	ufw allow "${SSH_PORT}/tcp" comment 'SSH'
	ufw allow 80/tcp comment 'HTTP - ACME challenge and HTTPS redirect'
	ufw allow 443/tcp comment 'HTTPS'
	ufw --force enable
	ufw status verbose
	echo "    NOTE: the VM's SSH port must really be ${SSH_PORT}."
else
	echo "==> Skipping host firewall (--no-firewall); use the cloud Security List."
fi

echo "==> Installing Docker Engine from Docker's apt repository"
install -m 0755 -d /etc/apt/keyrings
if [ ! -f /etc/apt/keyrings/docker.asc ]; then
	curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
	chmod a+r /etc/apt/keyrings/docker.asc
fi

# shellcheck disable=SC1091
CODENAME="$(. /etc/os-release && echo "${VERSION_CODENAME:-}")"
[ -n "$CODENAME" ] || {
	echo "cannot determine Ubuntu codename from /etc/os-release" >&2
	exit 1
}

cat >/etc/apt/sources.list.d/docker.list <<EOF
deb [arch=${ARCH} signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu ${CODENAME} stable
EOF

apt-get update -y
apt-get install -y --no-install-recommends \
	docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin

# If Docker was already running before this script (re-run), the ufw step above
# flushed its iptables chains. Restarting Docker re-creates them; containers
# come back through their restart policy.
DOCKER_WAS_ACTIVE=0
systemctl is-active --quiet docker && DOCKER_WAS_ACTIVE=1
systemctl enable --now docker
if [ "$DOCKER_WAS_ACTIVE" -eq 1 ]; then
	systemctl restart docker
fi

if [ "$DEPLOY_USER" != "root" ] && id "$DEPLOY_USER" >/dev/null 2>&1; then
	usermod -aG docker "$DEPLOY_USER"
	echo "    added '$DEPLOY_USER' to the docker group (log out/in for it to apply)"
fi

echo "==> Verifying installation"
docker --version
docker compose version
sqlite3 --version | awk '{print "sqlite3 " $1}'

echo
echo "Done. Next steps (as a normal user with docker group access):"
echo "  cd $(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
echo "  cp deploy/.env.example deploy/.env && chmod 600 deploy/.env"
echo "  # edit deploy/.env (hostname, ACME email, bcrypt hash)"
echo "  ./scripts/deploy.sh --with-duckdns   # omit the flag for a custom domain"
