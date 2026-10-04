/**
 * Story groups (≤ 100 from /story-groups) in a TanStack Table v9 grid: kind/service/endpoint
 * filters and the text search come from the URL, sorting is local. One row is selected (the
 * inspector shows it); ↑/↓ move the selection and Enter opens the group's sample story.
 */
import {
  columnFilteringFeature,
  createColumnHelper,
  createFilteredRowModel,
  createSortedRowModel,
  filterFn_equalsStringSensitive,
  filterFn_includesString,
  functionalUpdate,
  globalFilteringFeature,
  rowSortingFeature,
  sortFn_alphanumeric,
  tableFeatures,
  useTable,
} from '@tanstack/react-table'
import type { ColumnFiltersState, SortingState } from '@tanstack/react-table'
import { ArrowDown, ArrowUp, ChevronDown, Search, X } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import type { KeyboardEvent, ReactNode } from 'react'
import type { StoryGroup } from '../../api/types'
import type { HomeSearch, Since } from '../../app/search'
import { Spark } from '../../components/charts/Spark'
import { Button } from '../../components/ui/Button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../../components/ui/DropdownMenu'
import { EmptyState } from '../../components/ui/EmptyState'
import { cx } from '../../lib/cx'
import { ago } from '../../lib/format'
import { SINCE_SECS, bucketWord, denseSeries, endpointOf, peak, splitSummary } from './model'

const features = tableFeatures({
  rowSortingFeature,
  columnFilteringFeature,
  globalFilteringFeature,
  sortedRowModel: createSortedRowModel(),
  filteredRowModel: createFilteredRowModel(),
  sortFns: { alphanumeric: sortFn_alphanumeric },
  filterFns: { equalsStringSensitive: filterFn_equalsStringSensitive, includesString: filterFn_includesString },
})

const helper = createColumnHelper<typeof features, StoryGroup>()
const columns = helper.columns([
  helper.accessor('kind', { filterFn: 'equalsStringSensitive', enableGlobalFilter: false, enableSorting: false }),
  helper.accessor('summary', { id: 'title', sortFn: 'alphanumeric' }),
  helper.accessor('rc_service', { id: 'service', filterFn: 'equalsStringSensitive' }),
  helper.accessor(endpointOf, { id: 'endpoint', filterFn: 'equalsStringSensitive' }),
  helper.accessor('stories', { id: 'stories', enableGlobalFilter: false, sortDescFirst: true }),
  helper.accessor('last_seen_ns', { id: 'last', enableGlobalFilter: false, sortDescFirst: true }),
])

const SEARCHABLE = new Set(['title', 'service', 'endpoint'])
const DEFAULT_SORT: SortingState = [{ id: 'stories', desc: true }]
const SORT_WORD: Record<string, string> = { title: 'title', stories: 'stories', last: 'last seen' }
/** Stripe, group, trend, stories, last seen; trend and last seen drop out on phones. */
const COLS = 'grid-cols-[3px_minmax(0,1fr)_56px] sm:grid-cols-[3px_minmax(0,1fr)_120px_56px_76px]'

type FilterKey = 'kind' | 'service' | 'endpoint'

function filtersOf(search: HomeSearch): ColumnFiltersState {
  return (['kind', 'service', 'endpoint'] as const).flatMap((id) => (search[id] ? [{ id, value: search[id] }] : []))
}

function countBy(groups: readonly StoryGroup[], key: (g: StoryGroup) => string): Array<[string, number]> {
  const m = new Map<string, number>()
  for (const g of groups) m.set(key(g), (m.get(key(g)) ?? 0) + 1)
  return [...m].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
}

function KindMenu({ value, groups, onChange }: { value: HomeSearch['kind']; groups: readonly StoryGroup[]; onChange: (v: HomeSearch['kind']) => void }) {
  const n = (k: string) => groups.filter((g) => g.kind === k).length
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm" aria-label={`Kind filter: ${value ?? 'all kinds'}`}>
          {value === 'error' ? 'Errors' : value === 'slow' ? 'Slow' : 'All kinds'}
          <ChevronDown aria-hidden size={14} className="text-muted" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent>
        <DropdownMenuItem onSelect={() => onChange(undefined)}>All kinds</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => onChange('error')}>
          Errors <span className="ml-auto text-xs text-muted">{n('error')}</span>
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => onChange('slow')}>
          Slow <span className="ml-auto text-xs text-muted">{n('slow')}</span>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** "+ service" picker, or the active value as a chip with a remove button. */
function ValueChip({
  name,
  label,
  value,
  options,
  onChange,
}: {
  name: string
  label: string
  value: string | undefined
  options: Array<[string, number]>
  onChange: (v: string | undefined) => void
}) {
  if (value) {
    return (
      <span className="inline-flex h-[30px] max-w-[min(100%,320px)] items-center gap-1 rounded-control border border-field-line bg-field pl-2.5 pr-1 text-[13px]">
        <span className="text-muted">{label}:</span>
        <span className="min-w-0 truncate font-mono text-xs" title={value}>
          {value}
        </span>
        <button
          type="button"
          aria-label={`Remove ${label} filter`}
          onClick={() => onChange(undefined)}
          className="inline-flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-badge text-muted hover:bg-rail-active hover:text-ink"
        >
          <X aria-hidden size={13} />
        </button>
      </span>
    )
  }
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm" variant="dashed" disabled={options.length === 0}>
          + {name}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent className="max-h-80 max-w-[min(92vw,420px)] overflow-y-auto">
        <DropdownMenuLabel>{label}</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {options.map(([v, n]) => (
          <DropdownMenuItem key={v} onSelect={() => onChange(v)}>
            <span className="min-w-0 truncate font-mono text-xs">{v}</span>
            <span className="ml-auto pl-3 text-xs text-muted">{n}</span>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function SortHeader({
  id,
  sorting,
  onSort,
  className,
  children,
}: {
  id: string
  sorting: SortingState
  onSort: (id: string) => void
  className?: string
  children: ReactNode
}) {
  const s = sorting[0]
  const dir = s?.id === id ? (s.desc ? 'descending' : 'ascending') : 'none'
  return (
    <div role="columnheader" aria-sort={dir} className={className}>
      <button
        type="button"
        onClick={() => onSort(id)}
        className={cx(
          'inline-flex cursor-pointer items-center gap-1 rounded-badge text-[11px] uppercase tracking-[0.06em] hover:text-ink',
          dir === 'none' ? 'text-muted' : 'text-ink',
        )}
      >
        {children}
        {dir === 'descending' ? <ArrowDown aria-hidden size={11} /> : dir === 'ascending' ? <ArrowUp aria-hidden size={11} /> : null}
      </button>
    </div>
  )
}

export interface GroupsTableProps {
  groups: readonly StoryGroup[]
  search: HomeSearch
  since: Since
  /** When the groups were fetched: the end of the sparkline window and "last seen" reference. */
  nowMs: number
  onSearch: (patch: Partial<HomeSearch>) => void
  /** The selected group's fingerprint (falls back to the first visible row). */
  selected: string | undefined
  onSelect: (fingerprint: string) => void
  onOpen: (group: StoryGroup) => void
  /** Reports the visible rows in order, so the page can fall back to the top one. */
  onVisible: (groups: StoryGroup[]) => void
}

export function GroupsTable({ groups, search, since, nowMs, onSearch, selected, onSelect, onOpen, onVisible }: GroupsTableProps) {
  const [sorting, setSorting] = useState<SortingState>(DEFAULT_SORT)
  const columnFilters = useMemo(() => filtersOf(search), [search])
  const data = groups as StoryGroup[]
  const table = useTable({
    features,
    columns,
    data,
    getRowId: (g) => g.fingerprint,
    state: { sorting, columnFilters, globalFilter: search.q ?? '' },
    onSortingChange: setSorting,
    onColumnFiltersChange: (u) => {
      const next = functionalUpdate(u, columnFilters)
      const v = (id: FilterKey) => next.find((f) => f.id === id)?.value as string | undefined
      onSearch({ kind: v('kind') as HomeSearch['kind'], service: v('service'), endpoint: v('endpoint') })
    },
    onGlobalFilterChange: (u) => onSearch({ q: String(functionalUpdate(u, search.q ?? '')) || undefined }),
    globalFilterFn: 'includesString',
    getColumnCanGlobalFilter: (c) => SEARCHABLE.has(c.id),
    enableSortingRemoval: false,
  })
  const rows = table.getRowModel().rows
  const visible = useMemo(() => rows.map((r) => r.original), [rows])
  useEffect(() => onVisible(visible), [visible, onVisible])

  const services = useMemo(() => countBy(groups, (g) => g.rc_service), [groups])
  const endpoints = useMemo(() => countBy(groups, endpointOf), [groups])
  const win = SINCE_SECS[since]
  const current = visible.find((g) => g.fingerprint === selected) ?? visible[0]

  const rowRefs = useRef(new Map<string, HTMLDivElement>())
  const focusRow = (fp: string) => {
    const el = rowRefs.current.get(fp)
    el?.focus()
    el?.scrollIntoView({ block: 'nearest' })
  }
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    // Only rows navigate: Enter on a sort button must not open a story.
    if (!current || visible.length === 0 || (e.target as HTMLElement).getAttribute('role') !== 'row') return
    const i = visible.indexOf(current)
    let next: StoryGroup | undefined
    if (e.key === 'ArrowDown') next = visible[Math.min(visible.length - 1, i + 1)]
    else if (e.key === 'ArrowUp') next = visible[Math.max(0, i - 1)]
    else if (e.key === 'Home') next = visible[0]
    else if (e.key === 'End') next = visible.at(-1)
    else if (e.key === 'Enter') {
      e.preventDefault()
      onOpen(current)
      return
    } else return
    e.preventDefault()
    if (next) {
      onSelect(next.fingerprint)
      focusRow(next.fingerprint)
    }
  }
  const onSort = (id: string) => {
    const s = sorting[0]
    const desc = s?.id === id ? !s.desc : id !== 'title'
    setSorting([{ id, desc }])
  }
  const filtered = Boolean(search.kind || search.service || search.endpoint || search.q)
  const sortWord = SORT_WORD[sorting[0]?.id ?? 'stories'] ?? 'stories'

  return (
    <>
      <div className="flex flex-wrap items-center gap-2 border-b border-line px-4 py-2.5 text-muted">
        <KindMenu value={search.kind} groups={groups} onChange={(kind) => onSearch({ kind })} />
        <ValueChip name="service" label="Service" value={search.service} options={services} onChange={(service) => onSearch({ service })} />
        <ValueChip name="endpoint" label="Endpoint" value={search.endpoint} options={endpoints} onChange={(endpoint) => onSearch({ endpoint })} />
        <label className="flex h-[30px] min-w-[150px] flex-1 items-center gap-1.5 rounded-control border border-field-line bg-field px-2 shadow-inset sm:max-w-[240px]">
          <Search aria-hidden size={13} className="shrink-0" />
          <input
            type="search"
            aria-label="Search story groups"
            placeholder="Search groups"
            value={search.q ?? ''}
            onChange={(e) => table.setGlobalFilter(e.target.value)}
            className="min-w-0 flex-1 border-0 bg-transparent font-[inherit] text-[13px] text-ink outline-none placeholder:text-faint"
          />
        </label>
        <span className="ml-auto text-xs" aria-live="polite">
          {visible.length === groups.length ? groups.length : `${visible.length} of ${groups.length}`}{' '}
          {groups.length === 1 ? 'group' : 'groups'} · sorted by {sortWord}
        </span>
      </div>
      {visible.length === 0 ? (
        <EmptyState
          title="No groups match these filters"
          description={filtered ? 'Clear a filter or the search to see more.' : undefined}
          action={
            filtered ? (
              <Button size="sm" onClick={() => onSearch({ kind: undefined, service: undefined, endpoint: undefined, q: undefined })}>
                Clear filters
              </Button>
            ) : null
          }
        />
      ) : (
        <div
          role="grid"
          aria-label="Story groups"
          aria-rowcount={visible.length + 1}
          onKeyDown={onKeyDown}
          className="max-h-[min(62vh,600px)] overflow-y-auto overscroll-contain"
        >
          <div role="row" className={cx('sticky top-0 z-10 grid items-center gap-3.5 border-b border-line-soft bg-panel py-2 pr-4', COLS)}>
            <span role="columnheader" aria-label="Kind" />
            <SortHeader id="title" sorting={sorting} onSort={onSort}>
              Group
            </SortHeader>
            <div role="columnheader" className="hidden text-[11px] uppercase tracking-[0.06em] text-muted sm:block">
              Trend
            </div>
            <SortHeader id="stories" sorting={sorting} onSort={onSort} className="text-right">
              Stories
            </SortHeader>
            <SortHeader id="last" sorting={sorting} onSort={onSort} className="hidden text-right sm:block">
              Last seen
            </SortHeader>
          </div>
          {rows.map((r, i) => {
            const g = r.original
            const sel = g.fingerprint === current?.fingerprint
            const { title, detail } = splitSummary(g.summary)
            const values = denseSeries(g.buckets, g.bucket_secs, win, nowMs)
            const slow = g.kind === 'slow'
            return (
              <div
                key={r.id}
                ref={(el) => {
                  if (el) rowRefs.current.set(g.fingerprint, el)
                  else rowRefs.current.delete(g.fingerprint)
                }}
                role="row"
                aria-rowindex={i + 2}
                aria-selected={sel}
                tabIndex={sel ? 0 : -1}
                data-fingerprint={g.fingerprint}
                onClick={() => onSelect(g.fingerprint)}
                onDoubleClick={() => onOpen(g)}
                className={cx(
                  'tg-row grid cursor-pointer items-center gap-3.5 border-b border-line-soft py-3 pr-4 last:border-b-0 focus-visible:-outline-offset-2',
                  COLS,
                  sel ? 'bg-row-selected' : 'hover:bg-inner',
                )}
              >
                <span
                  role="gridcell"
                  aria-label={slow ? 'Slow' : 'Error'}
                  className={cx(
                    'self-stretch',
                    slow ? 'bg-slow' : 'bg-err',
                    sel && (slow ? 'shadow-glow-slow' : 'shadow-glow-err'),
                  )}
                />
                <span role="gridcell" className="flex min-w-0 flex-col gap-1">
                  <span className="flex min-w-0 items-center gap-2">
                    <span
                      className={cx(
                        'shrink-0 rounded-badge px-1.5 py-px font-mono text-[10.5px]',
                        slow ? 'bg-slow-soft text-slow' : 'bg-err-soft text-err',
                      )}
                    >
                      {g.kind}
                    </span>
                    <span className="truncate font-medium" title={g.summary}>
                      {title}
                    </span>
                  </span>
                  <span className="truncate text-xs text-muted">
                    {detail ? `${detail} · ` : ''}
                    <span className="font-mono">{endpointOf(g)}</span>
                  </span>
                </span>
                <span role="gridcell" className="hidden sm:block">
                  <Spark
                    values={values}
                    tone={slow ? 'slow' : 'err'}
                    label={`Stories per ${bucketWord(g.bucket_secs)} over ${since}, peak ${peak(values)}`}
                  />
                </span>
                <span role="gridcell" className="tabular text-right font-semibold">
                  {g.stories}
                </span>
                <span role="gridcell" className="hidden text-right text-xs text-muted sm:block">
                  {ago(g.last_seen_ns, nowMs)}
                </span>
              </div>
            )
          })}
        </div>
      )}
    </>
  )
}
