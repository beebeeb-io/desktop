// The popover mounted on its own, for tests/render-popover.mjs (task 1683 slice 3): the same component, CSS and theme
// tokens the app's `?window=popover` route mounts, without the rest of `main.tsx`.
import { createRoot } from 'react-dom/client'
import PopoverApp from '../../src/PopoverApp'
import '../../src/design.css'
import '../../src/popover.css'

document.documentElement.classList.add('popover-window')
createRoot(document.getElementById('root')!).render(<PopoverApp />)
