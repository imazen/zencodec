#!/usr/bin/env bash
# Fetch the sources the harness builds from paths:
# - zencodec at the rev every codec is compared on. Codecs reach zencodec
#   three ways (crates.io requirement, git rev, workspace git rev); path
#   patches are the one way to send all of them to a single copy.
# - zenbitmaps and IMIJ main, with shim.py's return-type shims (their main
#   branches implement the trait signatures from before zencodec PR 149).
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p vendor

# snapshot <dir> <repo url> <rev> [path inside the repo]
snapshot() {
  local dir=$1 url=$2 rev=$3 sub=${4:-}
  [ -f "vendor/$dir/.rev" ] && [ "$(cat "vendor/$dir/.rev")" = "$rev" ] && return 0
  rm -rf "vendor/$dir" vendor/.git-tmp
  git init -q vendor/.git-tmp
  git -C vendor/.git-tmp fetch -q --depth 1 "$url" "$rev"
  mkdir -p "vendor/$dir"
  if [ -n "$sub" ]; then
    git -C vendor/.git-tmp archive FETCH_HEAD "$sub" | tar -x -C "vendor/$dir" --strip-components=1
  else
    git -C vendor/.git-tmp archive FETCH_HEAD | tar -x -C "vendor/$dir"
  fi
  rm -rf vendor/.git-tmp
  echo "$rev" > "vendor/$dir/.rev"
}

snapshot zencodec https://github.com/imazen/zencodec 5ed2daded81407e7206fd8de6e8f1d93a5447b5b
fresh=0
[ -f vendor/zenbitmaps-main/.rev ] || fresh=1
[ -f vendor/imij-main/.rev ] || fresh=1
snapshot zenbitmaps-main https://github.com/imazen/zenbitmaps 7617e3523da60dd407dbe829b770034387529018
snapshot imij-main https://github.com/imazen/zqi 57af1aa90c7ee0b27ee6c14276d09b2bf7614d22 imij
[ "$fresh" = 1 ] && python3 shim.py vendor
exit 0
