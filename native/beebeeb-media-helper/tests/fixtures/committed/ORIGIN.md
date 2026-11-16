# he-aac.m4a

The one fixture that is committed and not generated.

- Made on 2026-11-15 from a generated two-tone signal: 440 Hz on the left, 660 Hz on the right, plus 14 kHz on both.
- Encoded by fdk-aac 2.0.3, built from its upstream source: `fdkaac --silent -p 5 -b 64` (HE-AAC v1).
- Committed because no public static ffmpeg carries an HE-AAC encoder, so `scripts/media-fixtures.sh` cannot make it.
  The script copies this file after checking its hash.
- SHA-256: `5c9dde438b06cd3ffb8c99bd975d1a16cb27c01324b93ed3c5775c2232139912`; size: 18214 bytes.
- Licence: the repository's licence. It is a generated signal; there is no third-party content in it.
