// The shared BBLogo mark.
export function BBLogo({ size = 16, light = false }: { size?: number; light?: boolean }) {
  return (
    <span
      style={{ fontSize: size }}
      className={`font-bold tracking-tight ${light ? 'text-white' : 'text-ink'}`}
    >
      beebeeb<span className="text-amber">.io</span>
    </span>
  )
}
