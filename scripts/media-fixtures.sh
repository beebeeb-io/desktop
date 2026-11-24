#!/usr/bin/env bash
# Generates one small file per format row of the media plan, plus manifest.tsv (hashes and references) into
# native/beebeeb-media-helper/tests/fixtures/. Run by the checkpoint lane once, on the build VM; its output is committed.
# Needs ffmpeg with libx264, libx265, libvpx, libvpx-vp9, libaom-av1 (or libsvtav1), mpeg2video, mpeg4, prores_ks, wmv2,
# wmav2, libmp3lame, aac, flac, alac, libopus, libvorbis, pcm_s16le, ac3, eac3: the BtbN static build has them all. No
# non-free encoder: HE-AAC is the one committed file tests/fixtures/committed/he-aac.m4a (see committed/ORIGIN.md).
# It downloads nothing: a missing tool stops it with exit 2.
set -euo pipefail

command -v ffmpeg >/dev/null 2>&1 || { echo "ffmpeg missing" >&2; exit 2; }
for tool in sha256sum od awk dd head stat mktemp; do
  command -v "$tool" >/dev/null 2>&1 || { echo "$tool missing" >&2; exit 2; }
done

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="$here/../native/beebeeb-media-helper/tests/fixtures"

# Before anything is written: every encoder the script uses is in this ffmpeg, and the committed HE-AAC file is the
# one whose hash is pinned here.
HE_AAC_SHA256=5c9dde438b06cd3ffb8c99bd975d1a16cb27c01324b93ed3c5775c2232139912
encoders="$(ffmpeg -hide_banner -encoders 2>/dev/null)"
av1=libaom-av1
printf '%s\n' "$encoders" | grep -qE "^ [VAS][A-Z.]{5} libaom-av1( |$)" || av1=libsvtav1
# The list is read from this script's own encoder arguments (`-c:v NAME` / `-c:a NAME`), never kept apart from them, so a
# changed ffmpeg line changes what is checked. The AV1 encoder is the one chosen above; the other is not needed.
used="$(grep -vE '^[[:space:]]*#' "${BASH_SOURCE[0]}" | grep -oE -e '-c:[va] [A-Za-z0-9_-]+' | awk '{print $2}' | grep -vxE 'libaom-av1|libsvtav1' | sort -u)"
[ -n "$used" ] || { echo "no encoder found in $0" >&2; exit 3; }
for enc_name in $used "$av1"; do
  printf '%s\n' "$encoders" | grep -qE "^ [VAS][A-Z.]{5} ${enc_name}( |$)" ||
    { echo "ffmpeg has no encoder $enc_name" >&2; exit 3; }
done
committed="$out/committed/he-aac.m4a"
[ -f "$committed" ] || { echo "$committed is missing" >&2; exit 4; }
[ "$(sha256sum "$committed" | cut -d' ' -f1)" = "$HE_AAC_SHA256" ] ||
  { echo "$committed is not the committed HE-AAC file (sha256 mismatch)" >&2; exit 4; }

mkdir -p "$out/ref"
cd "$out"

MAX_BYTES=$((300 * 1024))
W=320
H=180
FFQ=(-hide_banner -loglevel error -y -nostdin -bitexact)
V=(-f lavfi -i "testsrc2=size=${W}x${H}:rate=25:duration=2")
A=(-f lavfi -i "sine=frequency=440:sample_rate=48000:duration=2")
ST=(-ac 2 -ar 48000)

manifest=manifest.tsv
printf '#file\tkind\tcodec\texpect\tsha256\tref\n' >"$manifest"

# file -> sha256 of the Y planes of the first 3 frames as ffmpeg decodes them (the planes are also kept in ref/<file>.y,
# one per fixture, so the test can compare the helper's picture against them and not only a hash).
ref_video() {
  local file="$1" raw frame=$((W * H * 3 / 2)) ysize=$((W * H))
  raw="$(mktemp)"
  ffmpeg "${FFQ[@]}" -i "$file" -fps_mode passthrough -frames:v 3 -f rawvideo -pix_fmt yuv420p "$raw"
  : >"ref/$file.y"
  for i in 0 1 2; do
    dd if="$raw" bs="$frame" skip="$i" count=1 status=none | head -c "$ysize" >>"ref/$file.y"
  done
  command rm -f "$raw"
  # Three different pictures, not one repeated: a container without pts (AVI) makes ffmpeg's default CFR output repeat
  # picture 0. The test source changes every frame, so a repeat is a wrong reference.
  [ "$(for i in 0 1 2; do dd if="ref/$file.y" bs="$ysize" skip="$i" count=1 status=none | sha256sum; done | sort -u | wc -l)" -eq 3 ] ||
    { echo "$file: the first 3 reference frames are not 3 distinct pictures" >&2; exit 1; }
  [ "$(stat -c %s "ref/$file.y")" -eq $((ysize * 3)) ] || { echo "$file: fewer than 3 frames decoded" >&2; exit 1; }
  sha256sum "ref/$file.y" | cut -d' ' -f1
}

# file -> sha256 of the first second as s16 PCM, 48 kHz stereo
ref_lossless() {
  ffmpeg "${FFQ[@]}" -i "$1" -t 1 -f s16le "${ST[@]}" - | sha256sum | cut -d' ' -f1
}

# file -> RMS of the first second as f32 PCM, 48 kHz stereo, six digits
ref_rms() {
  ffmpeg "${FFQ[@]}" -i "$1" -t 1 -f f32le "${ST[@]}" - | od -An -v -t f4 -w4 |
    awk '{ s += $1 * $1; n++ } END { if (n == 0) exit 1; printf "%.6f", sqrt(s / n) }'
}

# row FILE KIND CODEC EXPECT REFKIND   (the file already exists)
row() {
  local file="$1" kind="$2" codec="$3" expect="$4" refkind="$5" ref size
  [ -s "$file" ] || { echo "$file: not written" >&2; exit 1; }
  size="$(stat -c %s "$file")"
  [ "$size" -le "$MAX_BYTES" ] || { echo "$file: $size bytes, over $MAX_BYTES" >&2; exit 1; }
  case "$refkind" in
    y) ref="$(ref_video "$file")" ;;
    pcm) ref="$(ref_lossless "$file")" ;;
    rms) ref="$(ref_rms "$file")" ;;
    none) ref="-" ;;
    *) echo "bad ref kind $refkind" >&2; exit 1 ;;
  esac
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$file" "$kind" "$codec" "$expect" "$(sha256sum "$file" | cut -d' ' -f1)" "$ref" >>"$manifest"
}

enc() { ffmpeg "${FFQ[@]}" "$@"; }

# ---- video (12) ----
enc "${V[@]}" -c:v libx264 -pix_fmt yuv420p -profile:v high -an h264.mp4
row h264.mp4 video h264 play y
enc "${V[@]}" -c:v libx264 -pix_fmt yuv420p -x264-params tff=1 -flags +ildct+ilme -an -f mpegts h264-interlaced.ts
row h264-interlaced.ts video h264 system none
enc "${V[@]}" -c:v libx264 -pix_fmt yuv420p10le -profile:v high10 -an h264-10bit.mp4
row h264-10bit.mp4 video h264 system none
enc "${V[@]}" -c:v libx265 -pix_fmt yuv420p -x265-params log-level=error -tag:v hvc1 -an hevc-main.mp4
row hevc-main.mp4 video hevc system y
enc "${V[@]}" -c:v libx265 -pix_fmt yuv420p10le -x265-params log-level=error -tag:v hvc1 -an hevc-main10.mov
row hevc-main10.mov video hevc system y
enc "${V[@]}" -c:v libvpx -b:v 300k -an vp8.webm
row vp8.webm video vp8 play y
enc "${V[@]}" -c:v libvpx-vp9 -b:v 300k -row-mt 0 -an vp9.webm
row vp9.webm video vp9 play y
if [ "$av1" = libaom-av1 ]; then
  enc "${V[@]}" -c:v libaom-av1 -cpu-used 8 -crf 40 -b:v 0 -an av1.mkv
else
  enc "${V[@]}" -c:v libsvtav1 -crf 40 -an av1.mkv
fi
row av1.mkv video av1 play y
enc "${V[@]}" -c:v mpeg2video -b:v 800k -an -f mpeg mpeg2.mpg
row mpeg2.mpg video mpeg2video unsupported y
enc "${V[@]}" -c:v mpeg4 -b:v 500k -an mpeg4p2.avi
row mpeg4p2.avi video mpeg4 refuse none
enc "${V[@]}" -t 1 -c:v prores_ks -profile:v 0 -an prores.mov
row prores.mov video prores unsupported y
enc "${V[@]}" -c:v wmv2 -b:v 500k -an wmv2.wmv
row wmv2.wmv video wmv2 refuse none

# ---- audio (11) ----
enc "${A[@]}" "${ST[@]}" -c:a libmp3lame -b:a 128k mp3.mp3
row mp3.mp3 audio mp3 play rms
enc "${A[@]}" "${ST[@]}" -c:a aac -b:a 128k aac-lc.m4a
row aac-lc.m4a audio aac system rms
# HE-AAC v1: no public ffmpeg encodes it; the file is committed (committed/ORIGIN.md), copied after the hash check above.
cp "$committed" he-aac.m4a
row he-aac.m4a audio aac system rms
enc "${A[@]}" "${ST[@]}" -c:a flac -sample_fmt s16 flac.flac
row flac.flac audio flac play pcm
enc "${A[@]}" "${ST[@]}" -c:a alac -sample_fmt s16p alac.m4a
row alac.m4a audio alac play pcm
enc "${A[@]}" "${ST[@]}" -c:a libopus -b:a 96k opus.ogg
row opus.ogg audio opus play rms
enc "${A[@]}" "${ST[@]}" -c:a libvorbis -q:a 4 vorbis.ogg
row vorbis.ogg audio vorbis play rms
enc "${A[@]}" "${ST[@]}" -t 1 -c:a pcm_s16le pcm.wav
row pcm.wav audio pcm_s16le play pcm
enc "${A[@]}" "${ST[@]}" -c:a wmav2 -b:a 128k wmav2.wma
row wmav2.wma audio wmav2 refuse none
enc "${A[@]}" "${ST[@]}" -c:a ac3 -b:a 192k ac3.ac3
row ac3.ac3 audio ac3 unsupported rms
enc "${A[@]}" "${ST[@]}" -c:a eac3 -b:a 192k eac3.mka
row eac3.mka audio eac3 system rms

# ---- containers (3): a video stream and an audio stream together ----
enc "${V[@]}" "${A[@]}" -map 0:v -map 1:a "${ST[@]}" -c:v libx264 -pix_fmt yuv420p -c:a aac -b:a 128k h264-aac.mkv
row h264-aac.mkv video h264 silent y
enc "${V[@]}" "${A[@]}" -map 0:v -map 1:a "${ST[@]}" -c:v libx264 -pix_fmt yuv420p -c:a libmp3lame -b:a 128k h264-mp3.avi
row h264-mp3.avi video h264 play y
enc "${V[@]}" "${A[@]}" -map 0:v -map 1:a "${ST[@]}" -c:v libx264 -pix_fmt yuv420p -c:a aac -b:a 128k -f mpegts h264-aac.ts
row h264-aac.ts video h264 silent y

printf 'Generated by scripts/media-fixtures.sh from ffmpeg'"'"'s test sources; no third-party content.\n' >LICENSE.txt

rows="$(grep -vc '^#' "$manifest")"
[ "$rows" -eq 26 ] || { echo "manifest has $rows rows, expected 26" >&2; exit 1; }
echo "media-fixtures: $rows rows written to $out"
