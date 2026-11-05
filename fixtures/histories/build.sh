#!/bin/sh
# Build local bare git repos from the acme fixtures (docs/HANDOVER.md §13.2,
# §13.6): target/fixtures/acme-infra.git and acme-app.git, each with a short
# history, so `firelight serve` can pull them over file:// with no network.
#   acme-infra: the .firelight/ folder plus terraform/access/, then a later
#     change to terraform/access/ only, so its access entries are maybe stale.
#   acme-app: the .firelight/ folder plus deploy/.
# Usage: fixtures/histories/build.sh [out-dir]   (default target/fixtures)
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
out=${1:-$root/target/fixtures}
mkdir -p "$out"
out=$(cd "$out" && pwd)

g() { # g <dir> <date> git args…
  d=$1 date=$2
  shift 2
  GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 \
    GIT_AUTHOR_DATE=$date GIT_COMMITTER_DATE=$date \
    git -C "$d" -c user.email=fixtures@acme.example -c user.name=Fixtures \
    -c commit.gpgsign=false -c init.defaultBranch=main "$@"
}

build() { # build <name> <extra-path> <first-date> [<later-file> <later-date>]
  name=$1 extra=$2 date=$3
  work=$(mktemp -d)
  rm -rf "$out/$name.git"
  git init -q --bare "$out/$name.git"
  g "$work" "$date" init -q
  cp -R "$root/fixtures/$name/.firelight" "$work/.firelight"
  mkdir -p "$work/$(dirname "$extra")"
  printf '# %s\n' "$extra" > "$work/$extra"
  g "$work" "$date" add -A
  g "$work" "$date" commit -q -m "chore: $name as of $date"
  if [ $# -ge 5 ]; then
    printf '# changed\n' >> "$work/$4"
    g "$work" "$5" add -A
    g "$work" "$5" commit -q -m "chore: change $4"
  fi
  g "$work" "$date" push -q "$out/$name.git" HEAD:main
  rm -rf "$work"
  echo "$out/$name.git"
}

build acme-infra terraform/access/main.tf 2026-10-01T10:00:00Z terraform/access/main.tf 2026-10-20T10:00:00Z
build acme-app deploy/app.yaml 2026-10-02T10:00:00Z
