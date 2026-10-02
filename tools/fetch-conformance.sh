#!/usr/bin/env bash
# Downloads the MPEG-2 video conformance bitstreams of ISO/IEC 13818-4:2004
# (main-profile and 422-profile), which ISO publishes among its Publicly
# Available Standards, and unpacks them into a directory for
# tests/conformance.rs:
#
#   tools/fetch-conformance.sh [dir]          # default: target/conformance
#   MPEG2_CONFORMANCE_DIR=target/conformance cargo test --release --test conformance
#
# The files are data: bitstreams, two decoding traces whose reconstructed
# samples the test compares, and one deliberately wrong reconstruction. They
# are not committed to this repository; ISO's licence terms govern them.
set -euo pipefail

dest="${1:-target/conformance}"
base="https://standards.iso.org/ittf/PubliclyAvailableStandards/ISO_IEC_13818-4_2004_Conformance_Testing/Video/bitstreams"

files=(
  422-profile/hhi/hhi_burst_422/hhi_burst_422_long.bits.gz
  422-profile/hhi/hhi_burst_422/hhi_burst_422_short.bits.gz
  422-profile/ibm/ibm_dp_intra_422/ibm_dp_intra_422.m2v.gz
  422-profile/sony/sony_422_id01-1/sony_422_id01-1.bs.gz
  422-profile/sony/sony_422_id03-1/sony_422_id03-1.bs.gz
  422-profile/sony/sony_422_id13-1/sony_422_id13-1.bs.gz
  422-profile/tek/Tek6-422-bigBpic/Tek6.bit.gz
  422-profile/tek/Tek7-422-smallSlices/Tek7.bit.gz
  422-profile/tek/Tek9-422-uniformVLC/Tek9.bit.gz
  main-profile/att/att_mismatch/att.bits.gz
  main-profile/ccett/mcp10ccett/mcp10ccett.bits.gz
  main-profile/chromatic/chroma_dct_type-1/test.mpg.gz
  main-profile/chromatic/chroma_dct_type-1/wrong.decoded.gz
  main-profile/compcore/ccm1/ccm1.mpg.gz
  main-profile/gi/gi4/video.bits.gz
  main-profile/gi/gi6/bit_stream.gz
  main-profile/gi/gi7/bit_stream.gz
  main-profile/gi/gi_9/bit_stream.gz
  main-profile/gi/gi_from_tape/gi_stream.gz
  main-profile/hhi/hhi_burst_long/hhi_burst_long.bits.gz
  main-profile/hhi/hhi_burst_short/hhi_burst_short.bits.gz
  main-profile/ibm/ibm-bw-v3/ibm-bw.BITS.gz
  main-profile/lep/bits_conf_lep_11/bits_conf_lep_11.bits.gz
  main-profile/mei/MEI.stream16.long/MEI.stream16.long.gz
  main-profile/mei/MEI.stream16v2/MEI.stream16v2.gz
  main-profile/mei/mei.2conftest.4f/mei_2stream.4f.gz
  main-profile/mei/mei.2conftest.60f.new/mei_2stream.60f.new.gz
  main-profile/nokia/nokia6/nokia6_dual.bit.gz
  main-profile/nokia/nokia6/nokia6_dual.trace.gz
  main-profile/nokia/nokia6/nokia6_dual_60.bit.gz
  main-profile/nokia/nokia_7/nokia7_dual.bit.gz
  main-profile/ntr/ntr_skipped_v3/ntr_skipped_v3.bits.gz
  main-profile/sony/sony-ct1/sony-ct1.bits.gz
  main-profile/sony/sony-ct2/sony-ct2.bits.gz
  main-profile/sony/sony-ct3/sony-ct3.bs.gz
  main-profile/sony/sony-ct4/sony-ct4.bs.gz
  main-profile/tceh/tceh_conf2/conf2.bits.gz
  main-profile/tcela/tcela-10-killer/tcela-10.bits.gz
  main-profile/tcela/tcela-14-bff-dp/tcela-14.bits.gz
  main-profile/tcela/tcela-14-bff-dp/tcela-14.short.bits.gz
  main-profile/tcela/tcela-15-stuffing/tcela-15.bits.gz
  main-profile/tcela/tcela-16-matrices/tcela-16.bits.gz
  main-profile/tcela/tcela-17-dots/tcela-17.bits.gz
  main-profile/tcela/tcela-17-dots/tcela-17.trace.gz
  main-profile/tcela/tcela-18-d-pict/tcela-18.bits.gz
  main-profile/tcela/tcela-19-wide/tcela-19.bits.gz
  main-profile/tcela/tcela-6-slices/tcela-6.bits.gz
  main-profile/tcela/tcela-7-slices/tcela-7.bits.gz
  main-profile/tcela/tcela-8-fp-dp/tcela-8.bits.gz
  main-profile/tcela/tcela-9-fp-dp/tcela-9.bits.gz
  main-profile/tek/Tek-5-long/conf4.bit.gz
  main-profile/tek/Tek-5.2/conf4.bit.gz
  main-profile/teracom/teracom_vlc4/teracom_vlc4.bin.gz
  main-profile/ti/TI_cl_2/TI_c1_2.bits.gz
  main-profile/toshiba/toshiba_DPall-0/toshiba_DPall-0.mpg.gz
  main-profile/twilight_zone/anonymous/mpeg_target_practice.mpg.gz
  main-profile/twilight_zone/mei/MEI.stream17.long/MEI.stream17.long.gz
  main-profile/twilight_zone/mei/MEI2.stream17/MEI2.stream17.gz
  main-profile/twilight_zone/tcela/tcela-11v2/tcela-11v2.bits.gz
  main-profile/twilight_zone/tcela/tcela-12/tcela-12.bits.gz
)

for f in "${files[@]}"; do
  out="$dest/${f%.gz}"
  [ -s "$out" ] && continue
  mkdir -p "$(dirname "$out")"
  curl -fsSL --retry 3 -o "$out.gz" "$base/$f"
  gunzip -f "$out.gz"
done
echo "conformance bitstreams in $dest (${#files[@]} files)"
