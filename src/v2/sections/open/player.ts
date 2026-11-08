// One audio element for the window, outside React, so playback carries on when the view is left (DW-1503, DW-1504).
// Beebeeb supplies title, artist and position through the media session; Windows draws its media controls.
let audio: HTMLAudioElement | null = null
let currentUrl: string | null = null
let current: { id: string; title: string; artist: string } | null = null

const el = () => (audio ??= new Audio())

export const playerState = () => ({ current, playing: !!audio && !audio.paused, elapsed: audio?.currentTime ?? 0 })

export function loadTrack(id: string, bytes: Uint8Array, meta: { title: string; artist: string }, mime = 'audio/mpeg'): void {
  const a = el()
  if (currentUrl) URL.revokeObjectURL(currentUrl)
  currentUrl = URL.createObjectURL(new Blob([bytes as BlobPart], { type: mime }))
  a.src = currentUrl
  current = { id, ...meta }
  if ('mediaSession' in navigator) navigator.mediaSession.metadata = new MediaMetadata({ title: meta.title, artist: meta.artist, album: 'Beebeeb' })
  void a.play().catch(() => {})
}

export const toggle = () => { const a = el(); if (a.paused) void a.play().catch(() => {}); else a.pause() }
export const seek = (s: number) => { el().currentTime = s }
export const onTime = (fn: () => void) => { const a = el(); a.addEventListener('timeupdate', fn); a.addEventListener('play', fn); a.addEventListener('pause', fn); a.addEventListener('ended', fn); return () => { a.removeEventListener('timeupdate', fn); a.removeEventListener('play', fn); a.removeEventListener('pause', fn); a.removeEventListener('ended', fn) } }
export const duration = () => (audio && Number.isFinite(audio.duration) ? audio.duration : 0)
