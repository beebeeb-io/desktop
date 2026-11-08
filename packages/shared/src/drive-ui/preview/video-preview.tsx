// The video preview player with frame-fit handling.
import { useEffect, useRef, useState, type ReactNode } from 'react'

interface VideoPreviewProps {
  /** A whole-file blob (the non-streaming paths). */
  blob?: Blob
  /** LD-0001 PR-B: a `/sw-stream/<id>` URL from the streaming session. */
  src?: string
  /** The hook wires the element's events and its network-error recovery. */
  onElement?: (el: HTMLVideoElement | null) => void
  /** The buffering/stalled pill (streaming only). */
  overlay?: ReactNode
}

interface VideoFrameFitInput {
  containerWidth: number
  containerHeight: number
  videoWidth?: number
  videoHeight?: number
}

interface VideoFrameFit {
  width: number
  height: number
  aspectRatio: string
}

function formatTime(seconds: number): string {
  const m = Math.floor(seconds / 60)
  const s = Math.floor(seconds % 60)
  return `${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
}

export function getVideoFrameFit({
  containerWidth,
  containerHeight,
  videoWidth,
  videoHeight,
}: VideoFrameFitInput): VideoFrameFit {
  const safeContainerWidth = Math.max(0, containerWidth)
  const safeContainerHeight = Math.max(0, containerHeight)
  const naturalWidth = videoWidth && videoWidth > 0 ? videoWidth : 16
  const naturalHeight = videoHeight && videoHeight > 0 ? videoHeight : 9
  const videoRatio = naturalWidth / naturalHeight

  if (safeContainerWidth <= 0 || safeContainerHeight <= 0) {
    return {
      width: 0,
      height: 0,
      aspectRatio: `${naturalWidth} / ${naturalHeight}`,
    }
  }

  if (safeContainerWidth / safeContainerHeight > videoRatio) {
    return {
      width: safeContainerHeight * videoRatio,
      height: safeContainerHeight,
      aspectRatio: `${naturalWidth} / ${naturalHeight}`,
    }
  }

  return {
    width: safeContainerWidth,
    height: safeContainerWidth / videoRatio,
    aspectRatio: `${naturalWidth} / ${naturalHeight}`,
  }
}

export function VideoPreview({ blob, src, onElement, overlay }: VideoPreviewProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const videoRef = useRef<HTMLVideoElement>(null)
  const [url, setUrl] = useState<string | null>(null)
  const [playing, setPlaying] = useState(false)
  const [currentTime, setCurrentTime] = useState(0)
  const [duration, setDuration] = useState(0)
  const [containerSize, setContainerSize] = useState({ width: 0, height: 0 })
  const [videoSize, setVideoSize] = useState<{ width: number; height: number } | null>(null)

  useEffect(() => {
    // Streaming: the session's URL IS the source; re-setting it (a recovered
    // network error) must not reset the player's own state, which the hook
    // restores from the element.
    if (src) {
      setUrl(src)
      return
    }
    if (!blob) {
      setUrl(null)
      return
    }
    const objectUrl = URL.createObjectURL(blob)
    setCurrentTime(0)
    setDuration(0)
    setPlaying(false)
    setVideoSize(null)
    setUrl(objectUrl)
    return () => URL.revokeObjectURL(objectUrl)
  }, [blob, src])

  useEffect(() => {
    const container = containerRef.current
    if (!container) return

    function updateSize(width: number, height: number) {
      setContainerSize((prev) => (
        prev.width === width && prev.height === height ? prev : { width, height }
      ))
    }

    const rect = container.getBoundingClientRect()
    updateSize(rect.width, rect.height)

    const observer = new ResizeObserver(([entry]) => {
      if (!entry) return
      updateSize(entry.contentRect.width, entry.contentRect.height)
    })
    observer.observe(container)

    return () => observer.disconnect()
  }, [url])

  const progress = duration > 0 ? (currentTime / duration) * 100 : 0
  const frameFit = getVideoFrameFit({
    containerWidth: containerSize.width,
    containerHeight: containerSize.height,
    videoWidth: videoSize?.width,
    videoHeight: videoSize?.height,
  })
  const hasMeasuredContainer = frameFit.width > 0 && frameFit.height > 0

  function togglePlay() {
    const video = videoRef.current
    if (!video) return
    if (video.paused) {
      video.play()
    } else {
      video.pause()
    }
  }

  function handleSeek(e: React.MouseEvent<HTMLDivElement>) {
    const video = videoRef.current
    if (!video || !duration) return
    const rect = e.currentTarget.getBoundingClientRect()
    const ratio = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width))
    video.currentTime = ratio * duration
  }

  function handleLoadedMetadata() {
    const video = videoRef.current
    if (!video) return
    setDuration(video.duration || 0)
    if (video.videoWidth > 0 && video.videoHeight > 0) {
      setVideoSize({ width: video.videoWidth, height: video.videoHeight })
    }
  }

  if (!url) return null

  return (
    <div ref={containerRef} className="flex h-full w-full items-center justify-center">
      <div
        className="relative overflow-hidden rounded-md bg-black"
        style={{
          width: hasMeasuredContainer ? frameFit.width : '100%',
          height: hasMeasuredContainer ? frameFit.height : '100%',
          maxWidth: '100%',
          maxHeight: '100%',
          aspectRatio: frameFit.aspectRatio,
          boxShadow: '0 20px 60px -10px rgba(0,0,0,0.4)',
        }}
      >
        <video
          ref={(el) => {
            videoRef.current = el
            onElement?.(el)
          }}
          src={url}
          preload={src ? 'metadata' : undefined}
          className="h-full w-full object-contain"
          onPlay={() => setPlaying(true)}
          onPause={() => setPlaying(false)}
          onTimeUpdate={() => setCurrentTime(videoRef.current?.currentTime ?? 0)}
          onLoadedMetadata={handleLoadedMetadata}
          onClick={togglePlay}
        />

        {/* Streaming pill: buffering / stalled (LD-0001 PR-B) */}
        {overlay && (
          <div className="pointer-events-none absolute left-1/2 top-3 -translate-x-1/2 rounded-full border border-line bg-paper/90 px-3 py-1.5 text-[11px] font-medium text-ink-2 shadow-1">
            {overlay}
          </div>
        )}

        {/* Play button overlay */}
        {!playing && (
          <button
            type="button"
            onClick={togglePlay}
            className="absolute left-1/2 top-1/2 flex h-16 w-16 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full border-[1.5px] border-white/80 bg-black/50 backdrop-blur-sm transition-transform hover:scale-105"
          >
            <div
              className="ml-1"
              style={{
                width: 0,
                height: 0,
                borderTop: '12px solid transparent',
                borderBottom: '12px solid transparent',
                borderLeft: '18px solid white',
              }}
            />
          </button>
        )}

        {/* Controls bar */}
        <div
          className="absolute inset-x-0 bottom-0 p-3.5"
          style={{
            background: 'linear-gradient(0deg, rgba(0,0,0,0.7), transparent)',
          }}
        >
          {/* Seek bar */}
          <div
            className="mb-2 flex cursor-pointer items-center gap-1"
            onClick={handleSeek}
            role="slider"
            aria-valuenow={currentTime}
            aria-valuemin={0}
            aria-valuemax={duration}
            tabIndex={0}
          >
            <div className="relative h-[3px] flex-1 rounded-sm bg-white/20">
              <div
                className="absolute inset-y-0 left-0 rounded-sm bg-amber"
                style={{ width: `${progress}%` }}
              />
              <div
                className="absolute top-1/2 h-[11px] w-[11px] -translate-x-1/2 -translate-y-1/2 rounded-full bg-amber"
                style={{ left: `${progress}%` }}
              />
            </div>
          </div>

          {/* Time display */}
          <div className="flex items-center gap-2.5 text-paper">
            <span className="font-mono text-[11px]">
              {formatTime(currentTime)} / {formatTime(duration)}
            </span>
          </div>
        </div>
      </div>
    </div>
  )
}
