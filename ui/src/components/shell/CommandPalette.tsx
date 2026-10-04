import { useQuery, keepPreviousData } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { Command } from 'cmdk'
import { Activity, ChartGantt, Clock, History, Moon, ScrollText, Server, TextAlignStart, Waypoints } from 'lucide-react'
import type { ReactNode } from 'react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { api } from '../../api/queries'
import { HEX32, SINCE_VALUES, sinceSearch } from '../../app/search'
import type { Since } from '../../app/search'
import { pushRecent, readRecent } from '../../lib/recent'
import type { RecentItem } from '../../lib/recent'
import { useTheme } from '../../theme/ThemeProvider'
import { NEXT_MODE } from '../../theme/theme'
import { DialogContent, DialogRoot } from '../ui/Dialog'
import { Kbd } from '../ui/Kbd'
import { useSince } from './TimeRange'

const DEBOUNCE_MS = 150

export const PAGES: readonly (RecentItem & { icon: typeof Server; keys: string })[] = [
  { kind: 'page', id: '/', label: 'Stories', icon: TextAlignStart, keys: 'g s' },
  { kind: 'page', id: '/traces', label: 'Traces', icon: ChartGantt, keys: 'g t' },
  { kind: 'page', id: '/map', label: 'Service map', icon: Waypoints, keys: 'g m' },
  { kind: 'page', id: '/logs/alerts', label: 'Log alerts', icon: ScrollText, keys: 'g l' },
  { kind: 'page', id: '/logs/templates', label: 'Log templates', icon: ScrollText, keys: '' },
  { kind: 'page', id: '/pipeline', label: 'Pipeline health', icon: Activity, keys: 'g p' },
]

/** Navigates to a palette item, keeping the current time range. */
export function useOpenItem() {
  const navigate = useNavigate()
  const since = useSince()
  return useCallback(
    (item: RecentItem) => {
      const s = sinceSearch(since)
      switch (item.kind) {
        case 'page':
          switch (item.id) {
            case '/traces':
              return void navigate({ to: '/traces', search: s })
            case '/map':
              return void navigate({ to: '/map', search: s })
            case '/logs/alerts':
              return void navigate({ to: '/logs/alerts', search: s })
            case '/logs/templates':
              return void navigate({ to: '/logs/templates', search: s })
            case '/pipeline':
              return void navigate({ to: '/pipeline', search: s })
            default:
              return void navigate({ to: '/', search: s })
          }
        case 'service':
          return void navigate({ to: '/map', search: { ...s, service: item.id } })
        case 'group':
          return void navigate({ to: '/', search: { ...s, group: item.id } })
        case 'template':
          return void navigate({ to: '/logs/templates/$templateId', params: { templateId: item.id }, search: s })
        case 'trace':
          return void navigate({ to: '/traces/$traceId', params: { traceId: item.id }, search: s })
      }
    },
    [navigate, since],
  )
}

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value)
  useEffect(() => {
    const t = window.setTimeout(() => setV(value), ms)
    return () => window.clearTimeout(t)
  }, [value, ms])
  return v
}

const itemClass =
  'flex cursor-pointer items-center gap-2.5 rounded-control px-2.5 py-2 text-ink data-[selected=true]:bg-rail-active data-[selected=true]:text-accent'
const headingClass =
  '[&_[cmdk-group-heading]]:px-2.5 [&_[cmdk-group-heading]]:pb-1 [&_[cmdk-group-heading]]:pt-2 [&_[cmdk-group-heading]]:text-xs [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-[0.06em] [&_[cmdk-group-heading]]:text-muted'

function Row({ icon, label, hint, right }: { icon: ReactNode; label: string; hint?: string; right?: ReactNode }) {
  return (
    <>
      <span aria-hidden className="shrink-0 text-muted">
        {icon}
      </span>
      <span className="min-w-0 flex-1 truncate" title={label}>
        {label}
      </span>
      {hint ? <span className="max-w-[40%] shrink-0 truncate text-xs text-muted">{hint}</span> : null}
      {right}
    </>
  )
}

export function CommandPalette({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  return (
    <DialogRoot open={open} onOpenChange={onOpenChange}>
      <DialogContent
        title="Command palette"
        bare
        className="top-[10vh]"
      >
        <PaletteBody onOpenChange={onOpenChange} />
      </DialogContent>
    </DialogRoot>
  )
}

/** Mounted only while the dialog is open, so the query and recents start fresh each time. */
function PaletteBody({ onOpenChange }: { onOpenChange: (o: boolean) => void }) {
  const [text, setText] = useState('')
  const [recent] = useState<RecentItem[]>(readRecent)
  const { mode, cycle } = useTheme()
  const navigate = useNavigate()
  const openItem = useOpenItem()

  const query = text.trim()
  const debounced = useDebounced(query, DEBOUNCE_MS)
  const searching = debounced.length > 0 && debounced.length <= 200
  const search = useQuery({
    ...api.search(debounced),
    enabled: searching,
    placeholderData: keepPreviousData,
    meta: { outage: false },
  })
  const result = searching ? search.data : undefined
  const needle = query.toLowerCase()
  const isTraceId = HEX32.test(query)
  const traceId = isTraceId ? query.toLowerCase() : (result?.trace_id ?? undefined)

  const pick = useCallback(
    (item: RecentItem) => {
      pushRecent(item)
      onOpenChange(false)
      openItem(item)
    },
    [onOpenChange, openItem],
  )
  const act = useCallback(
    (fn: () => void) => {
      onOpenChange(false)
      fn()
    },
    [onOpenChange],
  )

  const pages = useMemo(() => PAGES.filter((p) => !needle || p.label.toLowerCase().includes(needle)), [needle])
  const actions = useMemo(() => {
    const list: { id: string; label: string; icon: ReactNode; run: () => void }[] = [
      { id: 'theme', label: `Toggle theme (now ${mode}, next ${NEXT_MODE[mode]})`, icon: <Moon size={15} />, run: cycle },
      ...SINCE_VALUES.map((v: Since) => ({
        id: `since-${v}`,
        label: `Time range: ${v}`,
        icon: <Clock size={15} />,
        run: () =>
          void navigate({
            to: '.',
            search: (prev: Record<string, unknown>) => ({ ...prev, since: v === '1h' ? undefined : v }),
            replace: true,
          } as never),
      })),
    ]
    return list.filter((a) => !needle || a.label.toLowerCase().includes(needle) || 'action'.includes(needle))
  }, [mode, cycle, navigate, needle])

  const showRecent = !query && recent.length > 0
  return (
    <Command shouldFilter={false} label="Command palette" loop className="flex flex-col">
      <div className="border-b border-panel-line py-3 pl-4 pr-12">
        <Command.Input
          autoFocus
          value={text}
          onValueChange={setText}
          placeholder="Jump to service, trace id, template…"
          maxLength={200}
          className="h-9 w-full rounded-field border border-field-line bg-field px-3 text-ink shadow-inset placeholder:text-muted"
        />
      </div>
      <Command.List className={`max-h-[min(55vh,420px)] overflow-y-auto overscroll-contain p-2 ${headingClass}`}>
        <Command.Empty className="px-3 py-6 text-center text-muted">
          {search.isFetching ? 'Searching…' : search.isError ? 'Search is unavailable.' : 'No matches.'}
        </Command.Empty>

        {traceId ? (
          <Command.Group heading="Trace">
            <Command.Item
              value={`trace:${traceId}`}
              className={itemClass}
              onSelect={() => pick({ kind: 'trace', id: traceId, label: traceId, hint: 'Trace' })}
            >
              <Row icon={<ChartGantt size={15} />} label={`Open trace ${traceId}`} />
            </Command.Item>
          </Command.Group>
        ) : null}

        {showRecent ? (
          <Command.Group heading="Recent">
            {recent.map((r) => (
              <Command.Item key={`${r.kind}:${r.id}`} value={`recent:${r.kind}:${r.id}`} className={itemClass} onSelect={() => pick(r)}>
                <Row icon={<History size={15} />} label={r.label} hint={r.hint ?? r.kind} />
              </Command.Item>
            ))}
          </Command.Group>
        ) : null}

        {pages.length > 0 ? (
          <Command.Group heading="Pages">
            {pages.map((p) => (
              <Command.Item key={p.id} value={`page:${p.id}`} className={itemClass} onSelect={() => pick({ kind: p.kind, id: p.id, label: p.label })}>
                <Row
                  icon={<p.icon size={15} />}
                  label={p.label}
                  right={
                    p.keys ? (
                      <Kbd aria-hidden className="shrink-0">
                        {p.keys}
                      </Kbd>
                    ) : null
                  }
                />
              </Command.Item>
            ))}
          </Command.Group>
        ) : null}

        {result && result.services.length > 0 ? (
          <Command.Group heading="Services">
            {result.services.map((s) => (
              <Command.Item key={s} value={`service:${s}`} className={itemClass} onSelect={() => pick({ kind: 'service', id: s, label: s, hint: 'Service' })}>
                <Row icon={<Server size={15} />} label={s} hint="open on the map" />
              </Command.Item>
            ))}
          </Command.Group>
        ) : null}

        {result && result.groups.length > 0 ? (
          <Command.Group heading="Story groups">
            {result.groups.map((g) => (
              <Command.Item
                key={g.fingerprint}
                value={`group:${g.fingerprint}`}
                className={itemClass}
                onSelect={() => pick({ kind: 'group', id: g.fingerprint, label: g.summary, hint: `${g.kind} · ${g.stories} ${g.stories === 1 ? 'story' : 'stories'}` })}
              >
                <Row icon={<TextAlignStart size={15} />} label={g.summary} hint={`${g.kind} · ${g.stories}`} />
              </Command.Item>
            ))}
          </Command.Group>
        ) : null}

        {result && result.templates.length > 0 ? (
          <Command.Group heading="Templates">
            {result.templates.map((t) => (
              <Command.Item
                key={t.template_id}
                value={`template:${t.template_id}`}
                className={itemClass}
                onSelect={() => pick({ kind: 'template', id: t.template_id, label: t.template, hint: t.service })}
              >
                <Row icon={<ScrollText size={15} />} label={t.template} hint={t.service} />
              </Command.Item>
            ))}
          </Command.Group>
        ) : null}

        {actions.length > 0 ? (
          <Command.Group heading="Actions">
            {actions.map((a) => (
              <Command.Item key={a.id} value={`action:${a.id}`} className={itemClass} onSelect={() => act(a.run)}>
                <Row icon={a.icon} label={a.label} />
              </Command.Item>
            ))}
          </Command.Group>
        ) : null}
      </Command.List>
      <div className="flex items-center gap-3 border-t border-panel-line px-4 pt-2.5 text-xs text-muted">
        <span>
          <Kbd>↑</Kbd> <Kbd>↓</Kbd> move
        </span>
        <span>
          <Kbd>↵</Kbd> open
        </span>
        <span>
          <Kbd>esc</Kbd> close
        </span>
      </div>
    </Command>
  )
}
