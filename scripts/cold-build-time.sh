#!/bin/bash
# Cold build time: zencodec 0.1.26 (crates.io) vs main. Usage: zc-buildtime.sh <main-rev> <runs>
set -euo pipefail
rev=$1; runs=${2:-5}
root=~/tmp/zc-buildtime; mkdir -p $root; cd $root
mk() { mkdir -p $1/src; echo 'pub use zencodec;' > $1/src/lib.rs
  printf '[package]\nname = "zcbt-%s"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[dependencies]\n%s\n' "$1" "$2" > $1/Cargo.toml; }
mk published 'zencodec = "=0.1.26"'
mk main "zencodec = { git = \"https://github.com/imazen/zencodec\", rev = \"$rev\" }"
mk published-std 'zencodec = { version = "=0.1.26", features = ["std"] }'
mk main-std "zencodec = { git = \"https://github.com/imazen/zencodec\", rev = \"$rev\", features = [\"std\"] }"
for v in published main published-std main-std; do (cd $v && cargo fetch -q && cargo tree -q --depth 1 | head -12 > ../$v.tree); done
unit() { # seconds zencodec took, from the timings HTML
  python3 - "$1" <<'PY'
import re,sys,json,glob
f=sorted(glob.glob(sys.argv[1]+"/target/cargo-timings/cargo-timing-*.html"))[-1]
s=open(f).read(); m=re.search(r'const UNIT_DATA = (\[.*?\]);', s, re.S)
d=json.loads(m.group(1)); print(sum(u["duration"] for u in d if u["name"]=="zencodec"))
PY
}
echo "variant profile run wall_s zencodec_unit_s"
for i in $(seq 1 $runs); do for prof in dev release; do for v in published main published-std main-std; do
  (cd $v; rm -rf target; flag=""; [ $prof = release ] && flag="--release"
   s=$(date +%s.%N); cargo build -q $flag --timings 2>/dev/null; e=$(date +%s.%N)
   echo "$v $prof $i $(echo "$e - $s" | bc) $(unit .)")
done; done; done
