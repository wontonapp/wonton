#!/bin/sh
# Runs nginx-cutover.py and the nginx templates against a real nginx in a
# throwaway container (no network). Needs Docker; touches nothing on the host.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
entry=$(CDPATH= cd -- "$here/../.." && pwd)
tag=wonton-entry-cutover-test
docker build -q -t "$tag" "$here" >/dev/null
exec docker run --rm --name "wonton-entry-cutover-$$" --network none \
    -v "$entry:/src:ro" "$tag" python3 -B /src/tests/cutover/test_cutover.py
