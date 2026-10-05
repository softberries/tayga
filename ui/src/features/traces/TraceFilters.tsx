/**
 * Explorer filter bar: service and endpoint pickers, duration bounds (whole ms), errors only,
 * and the trace-id jump. Every value is written to the URL by the page.
 */
import { useNavigate } from '@tanstack/react-router'
import { ArrowRight, TriangleAlert } from 'lucide-react'
import { useId, useState } from 'react'
import type { ClipboardEvent, FormEvent, KeyboardEvent } from 'react'
import { HEX32, sinceSearch } from '../../app/search'
import type { Since, TracesSearch } from '../../app/search'
import { Button } from '../../components/ui/Button'
import { Combobox } from '../../components/ui/Combobox'
import { ToggleGroup } from '../../components/ui/ToggleGroup'
import { cx } from '../../lib/cx'

const field =
  'h-[30px] rounded-control border border-field-line bg-field px-2 font-[inherit] text-[13px] text-ink shadow-inset outline-none placeholder:text-faint focus-visible:border-accent'

/** A trace id as typed or pasted: surrounding whitespace dropped, lowercased; null if not 32 hex. */
export function parseTraceId(raw: string): string | null {
  const t = raw.trim()
  return HEX32.test(t) ? t.toLowerCase() : null
}

/** Jumps to `/traces/:id` on submit or on pasting a valid id; anything else gets an inline error. */
export function TraceJump({ since }: { since: Since }) {
  const navigate = useNavigate()
  const [text, setText] = useState('')
  const [error, setError] = useState(false)
  const errId = useId()
  const go = (id: string) => void navigate({ to: '/traces/$traceId', params: { traceId: id }, search: sinceSearch(since) })
  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    const id = parseTraceId(text)
    if (id) go(id)
    else setError(true)
  }
  const onPaste = (e: ClipboardEvent<HTMLInputElement>) => {
    const id = parseTraceId(e.clipboardData.getData('text'))
    if (id) {
      e.preventDefault()
      go(id)
    }
  }
  return (
    <form role="search" aria-label="Open a trace by id" onSubmit={onSubmit} className="flex min-w-0 flex-col gap-1">
      <div className="flex min-w-0 items-center gap-1.5">
        <input
          type="text"
          inputMode="text"
          spellCheck={false}
          autoComplete="off"
          aria-label="Trace id"
          aria-invalid={error || undefined}
          aria-describedby={error ? errId : undefined}
          placeholder="Paste a trace id"
          value={text}
          onChange={(e) => {
            setText(e.target.value)
            setError(false)
          }}
          onPaste={onPaste}
          className={cx(field, 'w-full min-w-0 font-mono text-xs sm:w-[300px]', error && 'border-err')}
        />
        <Button type="submit" size="sm" aria-label="Open trace">
          <ArrowRight aria-hidden size={14} />
        </Button>
      </div>
      {error ? (
        <p id={errId} role="alert" className="m-0 flex items-center gap-1.5 text-xs text-err">
          <TriangleAlert aria-hidden size={12} />
          Not a trace id: expected 32 hex characters.
        </p>
      ) : null}
    </form>
  )
}

const SCOPE = [
  { value: 'endpoint', label: 'As endpoint' },
  { value: 'anywhere', label: 'Anywhere in trace' },
] as const

/** A whole-ms bound as typed: '' clears it; anything but digits is invalid (NaN). */
function parseBound(s: string): number | undefined {
  const t = s.trim()
  if (t === '') return undefined
  return /^\d{1,8}$/.test(t) ? Number(t) : Number.NaN
}

function DurationBounds({ min, max, onChange }: { min?: number; max?: number; onChange: (min?: number, max?: number) => void }) {
  // Drafts while typing; committed on Enter or blur. When the URL changes (a commit, Clear
  // filters, back/forward), a draft that no longer matches it is replaced; the other field's
  // in-progress draft is kept.
  const text = (v?: number) => (v === undefined ? '' : String(v))
  const [lo, setLo] = useState(text(min))
  const [hi, setHi] = useState(text(max))
  const [seen, setSeen] = useState({ min, max })
  if (seen.min !== min || seen.max !== max) {
    setSeen({ min, max })
    if (seen.min !== min) setLo(text(min))
    if (seen.max !== max) setHi(text(max))
  }
  const errId = useId()
  const a = parseBound(lo)
  const b = parseBound(hi)
  const error = Number.isNaN(a) || Number.isNaN(b)
    ? 'Durations are whole milliseconds.'
    : a !== undefined && b !== undefined && a > b
      ? 'Min must not exceed max.'
      : null
  const commit = () => {
    if (!error && (a !== min || b !== max)) onChange(a, b)
  }
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Enter') commit()
  }
  const input = (label: string, value: string, set: (v: string) => void) => (
    <input
      type="text"
      inputMode="numeric"
      aria-label={label}
      aria-invalid={error ? true : undefined}
      aria-describedby={error ? errId : undefined}
      placeholder={label.startsWith('Min') ? 'min' : 'max'}
      value={value}
      onChange={(e) => set(e.target.value)}
      onBlur={commit}
      onKeyDown={onKey}
      className={cx(field, 'tabular w-[68px] text-right font-mono text-xs', error && 'border-err')}
    />
  )
  return (
    <div className="flex flex-col gap-1">
      <div role="group" aria-label="Duration (ms)" className="flex items-center gap-1.5 text-[13px] text-muted">
        <span>Duration</span>
        {input('Min duration (ms)', lo, setLo)}
        <span aria-hidden>–</span>
        {input('Max duration (ms)', hi, setHi)}
        <span className="text-xs">ms</span>
      </div>
      {error ? (
        <p id={errId} role="alert" className="m-0 text-xs text-err">
          {error}
        </p>
      ) : null}
    </div>
  )
}

export interface TraceFiltersProps {
  search: TracesSearch
  since: Since
  services: readonly string[]
  /** Endpoint names seen in the current results, with their row counts. */
  endpoints: ReadonlyMap<string, number>
  onSearch: (patch: Partial<TracesSearch>) => void
}

export function TraceFilters({ search, since, services, endpoints, onSearch }: TraceFiltersProps) {
  const endpointNames = [...endpoints.keys()].sort((a, b) => (endpoints.get(b) ?? 0) - (endpoints.get(a) ?? 0) || a.localeCompare(b))
  const filtered = Boolean(search.service || search.endpoint || search.min_ms !== undefined || search.max_ms !== undefined || search.errors)
  return (
    <div className="flex flex-wrap items-start gap-x-3 gap-y-2.5 px-4 py-3">
      <div className="flex min-w-0 flex-wrap items-start gap-2">
        <Combobox
          label="Service"
          value={search.service}
          options={services}
          // A picked service matches any span by default: most services never serve a
          // trace's endpoint, so "as endpoint" alone would usually find nothing.
          onChange={(service) => onSearch({ service, touched: service ? true : undefined, endpoint: undefined })}
          emptyText="No services"
        />
        {search.service ? (
          <ToggleGroup
            label="Service match"
            options={SCOPE}
            value={search.touched ? 'anywhere' : 'endpoint'}
            onValueChange={(v) => onSearch({ touched: v === 'anywhere' ? true : undefined, endpoint: undefined })}
          />
        ) : null}
        <Combobox
          label="Endpoint"
          value={search.endpoint}
          options={endpointNames}
          counts={endpoints}
          onChange={(endpoint) => onSearch({ endpoint })}
          emptyText="No endpoints in these results"
        />
        <DurationBounds
          min={search.min_ms}
          max={search.max_ms}
          onChange={(min_ms, max_ms) => onSearch({ min_ms, max_ms })}
        />
        <Button
          size="sm"
          aria-pressed={Boolean(search.errors)}
          onClick={() => onSearch({ errors: search.errors ? undefined : true })}
          className={search.errors ? 'border-err/60 bg-err-soft text-err hover:bg-err-soft' : undefined}
        >
          <span aria-hidden className="size-2 rounded-full bg-err" />
          Errors only
        </Button>
        {filtered ? (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => onSearch({ service: undefined, touched: undefined, endpoint: undefined, min_ms: undefined, max_ms: undefined, errors: undefined })}
          >
            Clear filters
          </Button>
        ) : null}
      </div>
      <div className="w-full min-w-0 sm:ml-auto sm:w-auto">
        <TraceJump since={since} />
      </div>
    </div>
  )
}
