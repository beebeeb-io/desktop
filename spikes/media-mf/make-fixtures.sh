#!/usr/bin/env bash
# Writes the spike's fixtures and manifest.tsv from ffmpeg's own test sources (no third-party content).
# Usage: FFMPEG=… FFPROBE=… FDKAAC=… [FDK_LIB=dir of libfdk-aac.so] make-fixtures.sh <out-dir>
# Video: testsrc2, 320x180, 25 fps, 2 s (M9's parameters). Audio: 48 kHz stereo, 2 s, two tones per channel
# (440 Hz left / 660 Hz right, plus 14 kHz in both) so the probe can tell a full HE-AAC decode (SBR keeps the
# 14 kHz tone, PS keeps left != right) from a core-only one.
# manifest.tsv columns: file kind(av|v|a) codec want(nv12|p010) frames(ffprobe count) ref_y0 note
# ref_y0 = SHA-256 of frame 0's luma as ffmpeg decodes it: 8-bit sources 320*180 bytes of yuv420p, 10-bit
# sources 320*180*2 bytes of p010le (what Media Foundation's P010 output holds).
set -euo pipefail
: "${FFMPEG:?}" "${FFPROBE:?}" "${FDKAAC:?}"
out=${1:?out dir}
mkdir -p "$out"
cd "$out"
rm -f manifest.tsv
ff() { "$FFMPEG" -hide_banner -loglevel error -y "$@"; }
VIN=(-f lavfi -i "testsrc2=s=320x180:r=25:d=2")
AIN=(-f lavfi -i "aevalsrc=0.4*sin(2*PI*440*t)+0.2*sin(2*PI*14000*t)|0.4*sin(2*PI*660*t)+0.2*sin(2*PI*14000*t):s=48000:d=2")
AV=(-map 0:v -map 1:a)
X265=(-x265-params log-level=error)

# --- in-box controls ---
ff "${VIN[@]}" "${AIN[@]}" "${AV[@]}" -c:v libx264 -pix_fmt yuv420p -profile:v high -bf 2 -g 25 -c:a aac -b:a 128k h264.mp4
ff "${AIN[@]}" -c:a aac -b:a 128k aac-lc.m4a
ff "${AIN[@]}" -c:a libmp3lame -b:a 128k mp3.mp3
ff "${VIN[@]}" "${AIN[@]}" "${AV[@]}" -c:v mpeg2video -q:v 4 -c:a mp2 -b:a 192k -f mpeg mpeg2.mpg
# --- the gap formats (decision 2540, D-MED-1) ---
ff "${VIN[@]}" "${AIN[@]}" "${AV[@]}" -c:v wmv2 -b:v 600k -c:a wmav2 -b:a 128k -f asf wmv2.wmv
ff "${AIN[@]}" -c:a wmav2 -b:a 128k -f asf wmav2.wma
ff "${VIN[@]}" "${AIN[@]}" "${AV[@]}" -c:v libxvid -vtag XVID -bf 2 -q:v 4 -c:a libmp3lame -b:a 128k mpeg4p2.avi
ff "${VIN[@]}" -c:v msmpeg4 -vtag DIV3 -q:v 4 msmpeg4v3.avi
ff "${VIN[@]}" "${AIN[@]}" "${AV[@]}" -c:v libx264 -pix_fmt yuv420p -flags +ildct+ilme -x264-params tff=1 -c:a aac -b:a 128k -f mpegts h264-interlaced.ts
ff "${VIN[@]}" -c:v libx264 -pix_fmt yuv420p10le -profile:v high10 h264-10bit.mp4
ff "${VIN[@]}" -c:v libx264 -pix_fmt yuv422p -profile:v high422 h264-422.mp4
ff -f lavfi -i "aevalsrc=0.4*sin(2*PI*440*t)+0.2*sin(2*PI*14000*t)|0.4*sin(2*PI*660*t)+0.2*sin(2*PI*14000*t):s=48000:d=2" -c:a pcm_s16le tones.wav
fdk() { if [ -n "${FDK_LIB:-}" ]; then LD_LIBRARY_PATH="$FDK_LIB" "$FDKAAC" "$@"; else "$FDKAAC" "$@"; fi; }
fdk --silent -p 5 -b 64 -o he-aac.m4a tones.wav
fdk --silent -p 29 -b 32 -o he-aac-v2.m4a tones.wav
rm -f tones.wav
# --- HEVC, required path (Guus 2026-11-15 17:30: HEVC on the system decoder) ---
# As an iPhone writes it: hvc1 sample entry, AAC audio, MOV (and MP4); Main (SDR) and Main 10 (HDR, HLG tags).
ff "${VIN[@]}" "${AIN[@]}" "${AV[@]}" -c:v libx265 "${X265[@]}" -pix_fmt yuv420p -tag:v hvc1 -c:a aac -b:a 128k hevc-main.mp4
ff "${VIN[@]}" "${AIN[@]}" "${AV[@]}" -c:v libx265 "${X265[@]}" -pix_fmt yuv420p -tag:v hvc1 -c:a aac -b:a 128k -f mov hevc-main.mov
HLG=(-color_primaries bt2020 -color_trc arib-std-b67 -colorspace bt2020nc)
ff "${VIN[@]}" "${AIN[@]}" "${AV[@]}" -c:v libx265 "${X265[@]}" -pix_fmt yuv420p10le "${HLG[@]}" -tag:v hvc1 -c:a aac -b:a 128k hevc-main10.mp4
ff "${VIN[@]}" "${AIN[@]}" "${AV[@]}" -c:v libx265 "${X265[@]}" -pix_fmt yuv420p10le "${HLG[@]}" -tag:v hvc1 -c:a aac -b:a 128k -f mov hevc-main10.mov
ff "${VIN[@]}" -c:v libx265 "${X265[@]}" -pix_fmt yuv420p -tag:v hev1 hevc-main-hev1.mp4
ff "${VIN[@]}" -c:v libx265 "${X265[@]}" -pix_fmt yuv422p10le -tag:v hvc1 hevc-422-10.mp4
# --- a Store-packaged decoder other than HEVC (the VP9 extension): does a packaged decoder load at all? ---
ff "${VIN[@]}" -c:v libvpx-vp9 -b:v 300k -deadline good -cpu-used 4 vp9.webm

frames() { "$FFPROBE" -v error -select_streams v:0 -count_frames -show_entries stream=nb_read_frames -of csv=p=0 "$1" | head -1; }
# frame 0's luma: decode one frame to a file first (a pipe cut short by `head` fails the run under pipefail).
luma() { # file pix_fmt bytes
  local raw
  raw=$(mktemp)
  "$FFMPEG" -hide_banner -loglevel error -y -i "$1" -map 0:v:0 -frames:v 1 -f rawvideo -pix_fmt "$2" "$raw"
  if [ "$(stat -c %s "$raw")" -lt "$3" ]; then echo short; else head -c "$3" "$raw" | sha256sum | cut -d' ' -f1; fi
  rm -f "$raw"
}
ref8() { luma "$1" yuv420p $((320 * 180)); }
ref10() { luma "$1" p010le $((320 * 180 * 2)); }
row() { # file kind codec want note
  local f=$1 kind=$2 codec=$3 want=$4 note=$5 n=- r=-
  if [ "$kind" != a ]; then
    n=$(frames "$f")
    if [ "$want" = p010 ]; then r=$(ref10 "$f"); else r=$(ref8 "$f"); fi
  fi
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$f" "$kind" "$codec" "$want" "$n" "$r" "$note" >> manifest.tsv
}
printf '# file\tkind\tcodec\twant\tframes\tref_y0\tnote\n' > manifest.tsv
row h264.mp4 av h264-high+aac-lc nv12 "in-box control"
row aac-lc.m4a a aac-lc nv12 "in-box control"
row mp3.mp3 a mp3 nv12 "in-box control"
row mpeg2.mpg av mpeg2+mp2 nv12 "in-box control"
row wmv2.wmv av wmv2+wmav2/asf nv12 "gap: WMV (WMV8), WMA, ASF"
row wmav2.wma a wmav2/asf nv12 "gap: WMA"
row mpeg4p2.avi av xvid-asp+mp3/avi nv12 "gap: MPEG-4 Part 2 (Xvid, B-frames)"
row msmpeg4v3.avi v divx3/avi nv12 "gap: MPEG-4 Part 2 Microsoft v3 (DivX 3)"
row h264-interlaced.ts av h264-interlaced+aac/ts nv12 "gap: interlaced H.264"
row h264-10bit.mp4 v h264-high10 p010 "gap: 10-bit H.264"
row h264-422.mp4 v h264-high422 nv12 "gap: 4:2:2 H.264"
row he-aac.m4a a he-aac-v1 nv12 "gap: full HE-AAC (SBR)"
row he-aac-v2.m4a a he-aac-v2 nv12 "gap: full HE-AAC v2 (SBR+PS)"
row hevc-main.mp4 av hevc-main+aac/mp4 nv12 "required: HEVC Main, MP4"
row hevc-main.mov av hevc-main+aac/mov nv12 "required: HEVC Main, MOV (iPhone SDR)"
row hevc-main10.mp4 av hevc-main10-hlg+aac/mp4 p010 "required: HEVC Main 10, MP4"
row hevc-main10.mov av hevc-main10-hlg+aac/mov p010 "required: HEVC Main 10, MOV (iPhone HDR)"
row hevc-main-hev1.mp4 v hevc-main-hev1/mp4 nv12 "HEVC Main, hev1 sample entry"
row hevc-422-10.mp4 v hevc-main422-10 p010 "gap: 4:2:2 HEVC"
row vp9.webm v vp9/webm nv12 "Store-packaged decoder control (VP9 extension)"
echo "fixtures: $(($(wc -l < manifest.tsv) - 1)) rows"
ls -l | awk 'NR>1 {print $5, $9}'
sha256sum ./*.* > SHA256SUMS
