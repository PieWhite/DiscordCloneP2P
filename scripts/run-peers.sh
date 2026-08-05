#!/bin/bash
# Start meerdere instanties naast elkaar op deze Mac, elk met een eigen datamap en
# poort. Voor snel handmatig testen zonder tweede machine — de macOS-tegenhanger van
# run-peers.ps1, zelfde poorten en configformaat.
#
#   ./scripts/run-peers.sh              # 2 instanties
#   ./scripts/run-peers.sh --count 3    # 3 instanties, volledige mesh
#   ./scripts/run-peers.sh --stop       # alles afsluiten
#   ./scripts/run-peers.sh --logs       # meterregels (deler|kijker) van alle instanties
#   ./scripts/run-peers.sh --release    # release-build
#
# De datamappen komen in ./.localpeers/ en worden bij elke start opnieuw opgebouwd,
# zodat je nooit met een halve staat van een vorige run zit. Anders dan op Windows
# blokkeert een draaiende instantie het bouwen niet; het stoppen vooraf is er alleen
# zodat de poorten vrij zijn.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
peer_root="$root/.localpeers"
base_port=41650
count=2
release=0
actie="start"

while [ $# -gt 0 ]; do
  case "$1" in
    --stop) actie="stop" ;;
    --logs) actie="logs" ;;
    --release) release=1 ;;
    --count) shift; count="$1" ;;
    *) echo "onbekende optie: $1" >&2; exit 2 ;;
  esac
  shift
done

if [ "$actie" = "stop" ]; then
  if pkill -x fitcom 2>/dev/null; then
    echo "instantie(s) gestopt."
  else
    echo "Er draait niets."
  fi
  exit 0
fi

if [ "$actie" = "logs" ]; then
  [ -d "$peer_root" ] || { echo "Nog niets gestart: draai eerst zonder --logs." >&2; exit 1; }
  # Nieuwste logbestand per instantie; tail zet er zelf '==> pad <==' boven.
  bestanden=()
  for d in "$peer_root"/peer*/logs; do
    nieuwste=$(ls -t "$d"/*.log 2>/dev/null | head -1) || true
    [ -n "${nieuwste:-}" ] && bestanden+=("$nieuwste")
  done
  [ ${#bestanden[@]} -gt 0 ] || { echo "Nog geen logbestanden." >&2; exit 1; }
  exec tail -f "${bestanden[@]}" | grep -E --line-buffered 'deler|kijker'
fi

if [ "$count" -lt 2 ]; then
  echo "Met minder dan 2 instanties valt er niets te testen." >&2
  exit 1
fi

# Bestaande instanties eerst weg: anders klapt het binden van de poort.
pkill -x fitcom 2>/dev/null || true
sleep 0.5

profiel=debug
build_args=(build -p fitcom)
if [ "$release" = 1 ]; then
  profiel=release
  build_args+=(--release)
fi

echo "Bouwen ($profiel)..."
cargo "${build_args[@]}"

exe="$root/target/$profiel/fitcom"
[ -x "$exe" ] || { echo "binary niet gevonden op $exe" >&2; exit 1; }

rm -rf "$peer_root"
mkdir -p "$peer_root"

for ((i = 0; i < count; i++)); do
  name="peer$((i + 1))"
  dir="$peer_root/$name"
  mkdir -p "$dir"

  # Elke instantie krijgt alle anderen als peer: volledige mesh, geen host.
  {
    echo "display_name = \"$name\""
    echo "control_port = $((base_port + i * 2))"
    echo "media_port   = $((base_port + i * 2 + 1))"
    for ((j = 0; j < count; j++)); do
      [ "$j" = "$i" ] && continue
      echo ""
      echo "[[peers]]"
      echo "address      = \"127.0.0.1\""
      echo "label        = \"peer$((j + 1))\""
      echo "control_port = $((base_port + j * 2))"
    done
  } > "$dir/config.toml"

  "$exe" --data-dir "$dir" >/dev/null 2>&1 &
  echo "$name gestart op poort $((base_port + i * 2))  ($dir)"
done

echo ""
echo "Meters: ./scripts/run-peers.sh --logs"
echo "Ruw:    tail -f $peer_root/peer1/logs/*.log"
echo "Stop:   ./scripts/run-peers.sh --stop"
