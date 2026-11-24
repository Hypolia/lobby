#!/usr/bin/env bash
set -euo pipefail

DATA_DIR="${RSPIGOT_DATA_DIR:-/data}"
TEMPLATE_DIR="${RSPIGOT_TEMPLATE_DIR:-/opt/hypolia/template}"
SERVER_PORT="${SERVER_PORT:-25565}"
MAX_PLAYERS="${SHEEPWARS_MAX_PLAYERS:-${TOWERWARS_MAX_PLAYERS:-${MAX_PLAYERS:-100}}}"
PLUGIN_BIND="${PLUGIN_GRPC_BIND:-127.0.0.1:25580}"

mkdir -p "${DATA_DIR}"

if [[ ! -f "${DATA_DIR}/bukkit.yml" && -f "${TEMPLATE_DIR}/bukkit.yml" ]]; then
  cp "${TEMPLATE_DIR}/bukkit.yml" "${DATA_DIR}/bukkit.yml"
fi
if [[ ! -f "${DATA_DIR}/permissions.yml" && -f "${TEMPLATE_DIR}/permissions.yml" ]]; then
  cp "${TEMPLATE_DIR}/permissions.yml" "${DATA_DIR}/permissions.yml"
fi
if [[ ! -f "${DATA_DIR}/server.properties" && -f "${TEMPLATE_DIR}/server.properties" ]]; then
  cp "${TEMPLATE_DIR}/server.properties" "${DATA_DIR}/server.properties"
fi

# Unpack world once (if not already populated by an initContainer) and copy arena.json
if [[ ! -d "${DATA_DIR}/world" ]]; then
  if [[ -f "${TEMPLATE_DIR}/world.zip" ]]; then
    unzip -q -o "${TEMPLATE_DIR}/world.zip" -d "${DATA_DIR}"
  elif [[ -d "${TEMPLATE_DIR}/world" ]]; then
    cp -R "${TEMPLATE_DIR}/world" "${DATA_DIR}/world"
  fi
fi
if [[ ! -f "${DATA_DIR}/arena.json" && -f "${TEMPLATE_DIR}/arena.json" ]]; then
  cp "${TEMPLATE_DIR}/arena.json" "${DATA_DIR}/arena.json"
fi

# Resolve Velocity forwarding secret (mounted Secret file or env var)
SECRET="${FORWARDING_SECRET:-}"
if [[ -z "${SECRET}" && -f /etc/hypolia/forwarding/secret ]]; then
  SECRET="$(tr -d '\r\n' < /etc/hypolia/forwarding/secret)"
fi
if [[ -z "${SECRET}" ]]; then
  echo "Erreur : FORWARDING_SECRET ou /etc/hypolia/forwarding/secret manquant." >&2
  exit 1
fi

# Render spigot.yml with Velocity forwarding secret
if [[ -f "${TEMPLATE_DIR}/spigot.yml.example" ]]; then
  cp "${TEMPLATE_DIR}/spigot.yml.example" "${DATA_DIR}/spigot.yml"
fi

python3 - "${DATA_DIR}" "${SECRET}" "${SERVER_PORT}" "${MAX_PLAYERS}" "${PLUGIN_BIND}" <<'PY'
import pathlib, re, sys

data_dir = pathlib.Path(sys.argv[1])
secret = sys.argv[2]
port = sys.argv[3]
max_players = sys.argv[4]
plugin_bind = sys.argv[5]

spigot = data_dir / "spigot.yml"
if spigot.exists():
    text = spigot.read_text()
    text = re.sub(
        r"(  velocity:\n\s+enabled:\s*)(?:true|false)(\n\s+secret:\s*).*",
        rf"\g<1>true\g<2>'{secret}'",
        text,
    )
    spigot.write_text(text)

props = data_dir / "server.properties"
if props.exists():
    lines = props.read_text().splitlines()
    overrides = {
        "server-ip": "0.0.0.0",
        "server-port": port,
        "max-players": max_players,
        "online-mode": "false",
    }
    seen = set()
    out = []
    for line in lines:
        if "=" in line and not line.lstrip().startswith("#"):
            k, _ = line.split("=", 1)
            if k in overrides:
                out.append(f"{k}={overrides[k]}")
                seen.add(k)
                continue
        out.append(line)
    for k, v in overrides.items():
        if k not in seen:
            out.append(f"{k}={v}")
    props.write_text("\n".join(out) + "\n")

host, grpc_port = plugin_bind.rsplit(":", 1)
plugins_yml = data_dir / "plugins.yml"
plugins_yml.write_text(
    f"bind: {plugin_bind}\n"
    f"host: {host}\n"
    f"port: {grpc_port}\n"
    f"token: ''\n"
    f"decision-ticks: 1\n"
    f"late-cancel-ticks: 20\n"
    f"order: []\n"
    f"priorities: {{}}\n"
)
PY

cd "${DATA_DIR}"
exec /usr/local/bin/rspigot --world-dir "${DATA_DIR}" "$@"
