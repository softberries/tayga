import { Search } from 'lucide-react'
import { useEffect, useState } from 'react'
import { DialogContent, DialogRoot } from '../ui/Dialog'
import { Kbd } from '../ui/Kbd'

/** Header search field that opens the ⌘K palette. The palette itself lands in Task 12. */
export function CommandTrigger() {
  const [open, setOpen] = useState(false)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setOpen((o) => !o)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  return (
    <DialogRoot open={open} onOpenChange={setOpen}>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-keyshortcuts="Meta+K Control+K"
        className="flex h-9 min-w-0 flex-[0_1_260px] cursor-pointer items-center gap-2 rounded-field border border-field-line bg-field px-2.5 text-muted shadow-inset hover:text-ink"
      >
        <Search size={14} strokeWidth={2} aria-hidden />
        <span className="flex-1 truncate text-left">Jump to service, trace id, template…</span>
        <Kbd>⌘K</Kbd>
      </button>
      <DialogContent title="Command palette" description="Search and jump to services, traces, stories and templates.">
        <p className="m-0 text-muted">The command palette is not built yet.</p>
      </DialogContent>
    </DialogRoot>
  )
}
