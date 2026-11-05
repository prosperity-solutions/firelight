#!/bin/sh
# The next release version, from Conventional Commit subjects since the last
# published release (docs/HANDOVER.md §15, maintenance-manager §16.1; D9).
#
# Prints two lines for $GITHUB_OUTPUT:
#   version=X.Y.Z     (without the v)
#   released=true|false
#
# Releasable: a `feat` or `fix` subject, or any breaking change (`type!:` or a
# `BREAKING CHANGE:` footer), since the newest `v*` tag (a tag exists only once
# its release is published). Bumps: feat → minor, fix → patch, breaking →
# minor below 1.0 and major from 1.0. The first release is v0.1.0.
# POSIX sh; needs only git. Usage: next-version.sh [rev] (default HEAD).
set -eu

rev=${1:-HEAD}
last=$(git describe --tags --abbrev=0 --match 'v[0-9]*' "$rev" 2>/dev/null || true)
range=$rev
[ -z "$last" ] || range="$last..$rev"

subjects=$(git log --format=%s "$range")
bodies=$(git log --format=%b "$range")

breaking=false feat=false fix=false
if printf '%s\n' "$subjects" | grep -Eq '^[a-z]+(\([^)]*\))?!: ' ||
  printf '%s\n' "$bodies" | grep -Eq '^BREAKING[ -]CHANGE: '; then
  breaking=true
fi
printf '%s\n' "$subjects" | grep -Eq '^feat(\([^)]*\))?!?: ' && feat=true
printf '%s\n' "$subjects" | grep -Eq '^fix(\([^)]*\))?!?: ' && fix=true

if [ "$breaking" = false ] && [ "$feat" = false ] && [ "$fix" = false ]; then
  released=false
else
  released=true
fi

if [ -z "$last" ]; then
  version=0.1.0
else
  v=${last#v}
  case $v in
    *[!0-9.]* | *..* | .* | *.)
      echo "next-version: the tag $last is not vX.Y.Z" >&2
      exit 1
      ;;
  esac
  major=${v%%.*}
  rest=${v#*.}
  minor=${rest%%.*}
  patch=${rest#*.}
  if [ "$breaking" = true ] && [ "$major" -ge 1 ]; then
    major=$((major + 1)) minor=0 patch=0
  elif [ "$breaking" = true ] || [ "$feat" = true ]; then
    minor=$((minor + 1)) patch=0
  else
    patch=$((patch + 1))
  fi
  version=$major.$minor.$patch
fi

echo "version=$version"
echo "released=$released"
