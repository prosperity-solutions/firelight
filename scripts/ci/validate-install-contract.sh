#!/bin/sh
# The installer's settings contract (docs/HANDOVER.md §11.1): every
# FIRELIGHT_INSTALL_* variable the script reads is described in its header
# and listed in docs/install.md, and nothing is documented that it doesn't
# read. Ported in spirit from outfit's tests/validate-install-contract.sh
# (MIT; see NOTICE).
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
script="$root/site/install.sh.in"
doc="$root/docs/install.md"

used=$(grep -v '^#' "$script" | grep -oE 'FIRELIGHT_INSTALL_[A-Z_]+' | sort -u)
header=$(grep '^#' "$script" | grep -oE 'FIRELIGHT_INSTALL_[A-Z_]+' | sort -u)
docs=$(grep -oE 'FIRELIGHT_INSTALL_[A-Z_]+' "$doc" | sort -u)

bad=0
for v in $used; do
  echo "$header" | grep -qx "$v" || { echo "install.sh reads $v but its header doesn't describe it"; bad=1; }
  echo "$docs" | grep -qx "$v" || { echo "install.sh reads $v but docs/install.md doesn't list it"; bad=1; }
done
for v in $header $docs; do
  echo "$used" | grep -qx "$v" || { echo "$v is documented but install.sh doesn't read it"; bad=1; }
done
[ "$bad" = 0 ] && echo "install contract: $(echo "$used" | wc -l | tr -d ' ') variables, all documented"
exit "$bad"
