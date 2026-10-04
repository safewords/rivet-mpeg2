#!/usr/bin/env bash
# Throughput figures: per-kernel timings, and decoding / encoding at 720p
# and 1080p on one thread and on several.
#
#   tools/bench.sh [conformance dir] [work dir] [threads]
#
# The clips are made by this crate's encoder from tcela-7 of the
# conformance suite (Mobile & Calendar, 720x480, 61 frames), scaled up
# bilinearly: 60 frames at quantiser_scale_code 4 with two B-pictures
# between references (the encoder's defaults). Each figure is the fastest
# of several runs. To compare with an older commit, build its
# examples/bench.rs (and run the same commands) — the clips must be the
# same files.
set -euo pipefail

conf="${1:-target/conformance}"
work="${2:-target/bench}"
threads="${3:-8}"
src="$conf/main-profile/tcela/tcela-7-slices/tcela-7.bits"
[ -s "$src" ] || tools/fetch-conformance.sh "$conf"
mkdir -p "$work"

cargo build --release --example bench --example kernels
bin="${CARGO_TARGET_DIR:-target}/release/examples"

"$bin/kernels"

for size in "1280 720" "1920 1080"; do
  set -- $size
  clip="$work/clip-$2.m2v"
  [ -s "$clip" ] || "$bin/bench" enc "$src" "$1" "$2" 60 1 1 "$clip"
  for t in 1 "$threads"; do
    "$bin/bench" dec "$clip" 7 "$t"
    "$bin/bench" enc "$src" "$1" "$2" 30 3 "$t"
  done
done
