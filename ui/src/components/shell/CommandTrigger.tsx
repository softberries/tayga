import { Search } from 'lucide-react'
import { useEffect, useState } from 'react'
import { CommandPalette } from './CommandPalette'
import { Kbd } from '../ui/Kbd'

/** Header search field that opens the ⌘K palette. */
export function CommandTrigger() {
  const [open, setOpen] = useState(false)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        // Not over another dialog (its own focus trap would fight the palette).
        if (!open && document.querySelector('[role="dialog"]')) return
        setOpen(!open)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open])

  return (
    <CommandPalette open={open} onOpenChange={setOpen}>
      <button
        type="button"
        aria-keyshortcuts="Meta+K Control+K"
        className="flex h-9 min-w-0 flex-[0_1_340px] sm:min-w-[260px] cursor-pointer items-center gap-2 rounded-field border border-field-line bg-field px-2.5 text-muted shadow-inset hover:text-ink"
      >
        <Search size={14} strokeWidth={2} aria-hidden />
        <span className="flex-1 truncate text-left">Jump to service, trace id, template…</span>
        <Kbd>⌘K</Kbd>
      </button>
    </CommandPalette>
  )
}
