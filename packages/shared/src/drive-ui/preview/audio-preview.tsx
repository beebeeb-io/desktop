// The audio preview player.
import { useEffect, useRef, useState } from 'react'

interface AudioPreviewProps {
  /** A whole-file blob (the non-streaming paths). */
  blob?: Blob
  /** LD-0001 PR-B: a `/sw-stream/<id>` URL from the streaming session. */
  src?: string
  filename: string
  /** The hook wires the element's events and its network-error recovery. */
  onElement?: (el: HTMLAudioElement | null) => void
}

function formatTime(seconds: number): string {
  const h = Math.floor(seconds / 3600)
  const m = Math.floor((seconds % 3600) / 60)
  const s = Math.floor(seconds % 60)
  if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
  return `${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
}

function pcmToWavBlob(audioBuffer: AudioBuffer): Blob {
  const numChannels = audioBuffer.numberOfChannels
  const sampleRate = audioBuffer.sampleRate
  const numFrames = audioBuffer.length
  const bytesPerSample = 2
  const dataSize = numFrames * numChannels * bytesPerSample
  const buffer = new ArrayBuffer(44 + dataSize)
  const view = new DataView(buffer)

  const writeStr = (offset: number, str: string) => {
    for (let i = 0; i < str.length; i++) view.setUint8(offset + i, str.charCodeAt(i))
  }

  writeStr(0, 'RIFF')
  view.setUint32(4, 36 + dataSize, true)
  writeStr(8, 'WAVE')
  writeStr(12, 'fmt ')
  view.setUint32(16, 16, true)
  view.setUint16(20, 1, true)
  view.setUint16(22, numChannels, true)
  view.setUint32(24, sampleRate, true)
  view.setUint32(28, sampleRate * numChannels * bytesPerSample, true)
  view.setUint16(32, numChannels * bytesPerSample, true)
  view.setUint16(34, bytesPerSample * 8, true)
  writeStr(36, 'data')
  view.setUint32(40, dataSize, true)

  let offset = 44
  for (let i = 0; i < numFrames; i++) {
    for (let ch = 0; ch < numChannels; ch++) {
      const sample = Math.max(-1, Math.min(1, audioBuffer.getChannelData(ch)[i]))
      view.setInt16(offset, sample * 0x7fff, true)
      offset += 2
    }
  }

  return new Blob([buffer], { type: 'audio/wav' })
}

export function AudioPreview({ blob, src, filename, onElement }: AudioPreviewProps) {
  const audioRef = useRef<HTMLAudioElement>(null)
  const [url, setUrl] = useState<string | null>(null)
  const [playing, setPlaying] = useState(false)
  const [currentTime, setCurrentTime] = useState(0)
  const [duration, setDuration] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const [converting, setConverting] = useState(false)
  const convertedRef = useRef(false)

  useEffect(() => {
    convertedRef.current = false
    // Streaming: the session's URL IS the source (LD-0001 PR-B).
    if (src) {
      setUrl(src)
      return
    }
    if (!blob) {
      setUrl(null)
      return
    }
    const objectUrl = URL.createObjectURL(blob)
    setUrl(objectUrl)
    return () => URL.revokeObjectURL(objectUrl)
  }, [blob, src])

  const progress = duration > 0 ? (currentTime / duration) * 100 : 0
  const ext = filename.split('.').pop()?.toUpperCase() ?? 'Audio'

  async function decodeAndConvert() {
    if (convertedRef.current) return
    // The convert path decodes a whole file, so it has no meaning for a
    // stream: a streaming element error is the hook's decision (row 6/7).
    if (!blob) return
    convertedRef.current = true
    setConverting(true)
    setError(null)
    try {
      const arrayBuffer = await blob.arrayBuffer()
      const audioCtx = new AudioContext()
      const audioBuffer = await audioCtx.decodeAudioData(arrayBuffer)
      await audioCtx.close()
      const wavBlob = pcmToWavBlob(audioBuffer)
      const wavUrl = URL.createObjectURL(wavBlob)
      if (url) URL.revokeObjectURL(url)
      setUrl(wavUrl)
      setDuration(audioBuffer.duration)
    } catch {
      setError('Could not decode this audio file')
    } finally {
      setConverting(false)
    }
  }

  function togglePlay() {
    const audio = audioRef.current
    if (!audio) return
    if (audio.paused) {
      audio.play().catch(() => setError('Playback failed'))
    } else {
      audio.pause()
    }
  }

  function handleSeek(e: React.MouseEvent<HTMLDivElement>) {
    const audio = audioRef.current
    if (!audio || !duration) return
    const rect = e.currentTarget.getBoundingClientRect()
    const ratio = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width))
    audio.currentTime = ratio * duration
  }

  if (!url) return null

  return (
    <div className="flex w-full max-w-[480px] flex-col items-center gap-5">
      {/* Disc icon */}
      <div
        className="flex h-32 w-32 items-center justify-center rounded-full border border-line bg-paper-2"
        style={{ boxShadow: '0 8px 32px -8px rgba(0,0,0,0.2)' }}
      >
        <div className="flex h-20 w-20 items-center justify-center rounded-full border border-line/60 bg-paper">
          <span className="font-mono text-[11px] font-medium text-ink-3">{ext}</span>
        </div>
      </div>

      {/* Player card */}
      <div className="w-full rounded-xl border border-line bg-paper p-5" style={{ boxShadow: '0 4px 20px -4px rgba(0,0,0,0.12)' }}>
        <audio
          ref={(el) => {
            audioRef.current = el
            onElement?.(el)
          }}
          src={url}
          preload="metadata"
          onPlay={() => setPlaying(true)}
          onPause={() => setPlaying(false)}
          onEnded={() => setPlaying(false)}
          onTimeUpdate={() => setCurrentTime(audioRef.current?.currentTime ?? 0)}
          onLoadedMetadata={() => setDuration(audioRef.current?.duration ?? 0)}
          onError={() => {
            if (src) return // streaming: the hook decides (rows 6-7)
            if (!convertedRef.current) {
              decodeAndConvert()
            } else {
              setError('Could not decode this audio file')
            }
          }}
        />

        {converting ? (
          <div className="flex flex-col items-center gap-2 py-2">
            <svg className="h-5 w-5 animate-spin text-amber" viewBox="0 0 24 24" fill="none">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
            </svg>
            <span className="text-[12px] text-ink-3">Decoding {ext} for playback...</span>
          </div>
        ) : error ? (
          <p className="text-center text-sm text-red">{error}</p>
        ) : (
          <>
            {/* Track name */}
            <p className="mb-4 truncate text-center text-[13px] font-medium text-ink">
              {filename}
            </p>

            {/* Seek bar */}
            <div
              className="group mb-3 flex cursor-pointer items-center"
              onClick={handleSeek}
              role="slider"
              aria-label="Seek"
              aria-valuenow={currentTime}
              aria-valuemin={0}
              aria-valuemax={duration}
              tabIndex={0}
            >
              <div className="relative h-[4px] flex-1 rounded-full bg-ink/10">
                <div
                  className="absolute inset-y-0 left-0 rounded-full bg-amber"
                  style={{ width: `${progress}%` }}
                />
                <div
                  className="absolute top-1/2 h-3 w-3 -translate-x-1/2 -translate-y-1/2 rounded-full bg-amber opacity-0 transition-opacity group-hover:opacity-100"
                  style={{ left: `${progress}%` }}
                />
              </div>
            </div>

            {/* Time + controls */}
            <div className="flex items-center justify-between">
              <span className="font-mono text-[11px] text-ink-3">
                {formatTime(currentTime)}
              </span>

              <button
                type="button"
                onClick={togglePlay}
                className="flex h-10 w-10 items-center justify-center rounded-full bg-amber text-paper transition-transform hover:scale-105 active:scale-95"
              >
                {playing ? (
                  <svg width="14" height="16" viewBox="0 0 14 16" fill="currentColor">
                    <rect x="1" y="0" width="4" height="16" rx="1" />
                    <rect x="9" y="0" width="4" height="16" rx="1" />
                  </svg>
                ) : (
                  <svg width="14" height="16" viewBox="0 0 14 16" fill="currentColor" className="ml-0.5">
                    <path d="M1 1.3a1 1 0 0 1 1.5-.86l11 7a1 1 0 0 1 0 1.72l-11 7A1 1 0 0 1 1 15.3V1.3Z" />
                  </svg>
                )}
              </button>

              <span className="font-mono text-[11px] text-ink-3">
                {duration > 0 ? formatTime(duration) : '--:--'}
              </span>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
