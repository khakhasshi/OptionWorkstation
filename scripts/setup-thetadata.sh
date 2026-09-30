#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PYTHON="${PYTHON:-python3}"
VENV="${OPTION_WORKSTATION_THETADATA_VENV:-$ROOT/.venv-thetadata}"

"$PYTHON" -c 'import sys; raise SystemExit(0 if sys.version_info >= (3, 12) else "ThetaData live mode requires Python 3.12+")'
"$PYTHON" -m venv "$VENV"
"$VENV/bin/python" -m pip install --upgrade pip
"$VENV/bin/python" -m pip install --requirement "$ROOT/requirements-thetadata.txt"
"$VENV/bin/python" -c 'from thetadata import ThetaClient; print("ThetaData SDK import passed")'

printf 'ThetaData adapter environment ready: %s\n' "$VENV/bin/python"
