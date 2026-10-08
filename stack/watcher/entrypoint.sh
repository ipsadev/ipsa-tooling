#!/bin/bash
set -euo pipefail

CFG=${WATCHER_CONFIG:-/etc/stellar/stellar-core.cfg}
DB=/data/stellar.db

if [ ! -f "$DB" ]; then
  echo "[watcher] first run: initialising database"
  stellar-core --conf "$CFG" new-db
  echo "[watcher] catching up to the present"
  stellar-core --conf "$CFG" catchup current/2
else
  echo "[watcher] existing database found; skipping init"
fi

echo "[watcher] following the network"
exec stellar-core --conf "$CFG" run
