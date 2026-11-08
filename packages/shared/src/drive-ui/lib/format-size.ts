/**
 * One binary-unit size format for preview surfaces (LD-0001 PR-C review,
 * reuse item): the file info rail and the progress copy share this, so a
 * transfer never says "12.4 MB" next to an info panel saying "13 MB".
 *
 * The ladder itself is core's (`formatBinary`, W-P3-4); this name is kept for
 * the surfaces that already import it.
 */
export { formatBinarySize as formatSize } from './format'
