// The PDF preview.
import { useEffect, useMemo } from 'react'

interface PdfPreviewProps {
  blob: Blob
}

export function PdfPreview({ blob }: PdfPreviewProps) {
  // Two URLs: the bare blob: URL (revoke target on unmount) and the same
  // URL with `#view=FitH` appended (fed to the iframe). `FitH` tells
  // Chrome's PDFium viewer to scale the page to fit the iframe's width
  // — without it the PDF renders at native size and a narrow viewport
  // (phones, side panels) shows only the top-left corner.
  const blobUrl = useMemo(() => {
    const pdfBlob = new Blob([blob], { type: 'application/pdf' })
    return URL.createObjectURL(pdfBlob)
  }, [blob])

  useEffect(() => {
    return () => URL.revokeObjectURL(blobUrl)
  }, [blobUrl])

  return (
    <iframe
      src={`${blobUrl}#view=FitH`}
      sandbox="allow-popups allow-scripts"
      // The parent only sets `min-h-[45vh]` on mobile (not an explicit
      // height), so `h-full` resolves to `auto` and the iframe falls back
      // to the 150px browser default. Force a usable mobile height with
      // min-h-[80vh]; on md+ the parent has flex-1 so h-full takes over.
      className="block w-full h-full min-h-[80vh] md:min-h-0 border-0 rounded"
      title="PDF preview"
    />
  )
}
