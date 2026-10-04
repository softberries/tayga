import { useNavigate } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { sinceSearch } from '../../app/search'
import { DialogContent, DialogRoot } from '../ui/Dialog'
import { Kbd } from '../ui/Kbd'
import { useSince } from './TimeRange'

const GO = {
  s: { to: '/', label: 'Stories' },
  t: { to: '/traces', label: 'Traces' },
  m: { to: '/map', label: 'Service map' },
  l: { to: '/logs/alerts', label: 'Logs and templates' },
  p: { to: '/pipeline', label: 'Pipeline health' },
} as const

const CHORD_MS = 1000

/** True where typing must not trigger shortcuts: fields, editors and open dialogs/menus. */
function typing(e: KeyboardEvent): boolean {
  const t = e.target
  if (!(t instanceof HTMLElement)) return false
  if (t.isContentEditable || t.closest('input, textarea, select, [contenteditable="true"]')) return true
  return document.querySelector('[role="dialog"], [role="menu"]') !== null
}

/**
 * `g` then s/t/m/l/p goes to a section; `?` lists the shortcuts. Ignored while typing, with
 * a modifier held, and while a dialog is open (so the palette's input keeps every key).
 */
export function Shortcuts() {
  const navigate = useNavigate()
  const since = useSince()
  const [help, setHelp] = useState(false)
  const armed = useRef<number | null>(null)

  useEffect(() => {
    const disarm = () => {
      if (armed.current !== null) window.clearTimeout(armed.current)
      armed.current = null
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey || e.defaultPrevented || e.repeat || typing(e)) return
      if (armed.current !== null) {
        disarm()
        const dest = GO[e.key as keyof typeof GO]
        if (dest) {
          e.preventDefault()
          void navigate({ to: dest.to, search: sinceSearch(since) })
          return
        }
      }
      if (e.key === 'g') {
        armed.current = window.setTimeout(disarm, CHORD_MS)
      } else if (e.key === '?') {
        e.preventDefault()
        setHelp(true)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('keydown', onKey)
      disarm()
    }
  }, [navigate, since])

  return (
    <DialogRoot open={help} onOpenChange={setHelp}>
      <DialogContent title="Keyboard shortcuts" description="Shortcuts do not fire while you are typing in a field.">
        <dl className="m-0 grid grid-cols-[auto_1fr] items-center gap-x-5 gap-y-2.5">
          <Row keys={['⌘', 'K']} alt={['Ctrl', 'K']} label="Open the command palette" />
          {Object.entries(GO).map(([k, d]) => (
            <Row key={k} keys={['g', k]} label={`Go to ${d.label}`} />
          ))}
          <Row keys={['?']} label="Show this list" />
          <Row keys={['Esc']} label="Close a dialog or the palette" />
        </dl>
      </DialogContent>
    </DialogRoot>
  )
}

function Row({ keys, alt, label }: { keys: string[]; alt?: string[]; label: string }) {
  return (
    <>
      <dt className="flex items-center gap-1">
        {keys.map((k, i) => (
          <Kbd key={i}>{k}</Kbd>
        ))}
        {alt ? (
          <>
            <span className="px-1 text-xs text-muted">or</span>
            {alt.map((k, i) => (
              <Kbd key={i}>{k}</Kbd>
            ))}
          </>
        ) : null}
      </dt>
      <dd className="m-0">{label}</dd>
    </>
  )
}
