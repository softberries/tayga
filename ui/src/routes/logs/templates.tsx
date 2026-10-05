/**
 * `/logs/templates`: the log templates seen in the window, with a service filter and a
 * debounced text search. Both live in the URL; the list refreshes in live mode.
 */
import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { Search } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { api } from '../../api/queries'
import { useLiveInterval } from '../../app/live'
import type { LogTemplatesSearch } from '../../app/search'
import { useSince } from '../../components/shell/TimeRange'
import { Button } from '../../components/ui/Button'
import { Card } from '../../components/ui/Card'
import { Combobox } from '../../components/ui/Combobox'
import { EmptyState } from '../../components/ui/EmptyState'
import { Skeleton } from '../../components/ui/Skeleton'
import { RefreshNote, loadFailed } from '../../components/ui/StaleNote'
import { TemplatesTable } from '../../features/logs/TemplatesTable'
import type { TemplateSort } from '../../features/logs/model'
import { ErrorBanner } from '../../features/stories/ErrorBanner'
import { Reveal } from '../../components/ui/Reveal'

/** Search results are requested this long after the last keystroke. */
export const SEARCH_DEBOUNCE_MS = 300
/** The API's cap on rows. */
const API_LIMIT = 200

/**
 * Text for an input whose value lives in the URL: typing updates the field at once and the
 * URL after `ms` of quiet; a change from elsewhere (clear filters, back) replaces the text.
 */
function useDebouncedParam(url: string | undefined, commit: (v: string | undefined) => void, ms: number) {
  const [text, setText] = useState(url ?? '')
  const [seen, setSeen] = useState(url)
  const [committed, setCommitted] = useState(url)
  if (seen !== url) {
    setSeen(url)
    // Our own commit coming back must not overwrite what was typed since.
    if (url !== committed) {
      setText(url ?? '')
      setCommitted(url)
    }
  }
  useEffect(() => {
    const v = text.trim() || undefined
    if (v === url) return
    const id = setTimeout(() => {
      setCommitted(v)
      commit(v)
    }, ms)
    return () => clearTimeout(id)
  }, [text, url, commit, ms])
  return [text, setText] as const
}

function TableSkeleton() {
  return (
    <div aria-busy="true" aria-label="Loading log templates">
      {Array.from({ length: 8 }, (_, i) => (
        <div key={i} className="flex items-center gap-4 border-b border-line-soft px-4 py-3 last:border-b-0">
          <Skeleton className="h-3.5 w-24" />
          <Skeleton className="h-3.5 flex-1" />
          <Skeleton className="hidden h-3.5 w-28 sm:block" />
        </div>
      ))}
    </div>
  )
}

export function LogTemplatesPage() {
  const since = useSince()
  const search = useSearch({ from: '/logs/templates/' })
  const navigate = useNavigate({ from: '/logs/templates/' })
  const refetchInterval = useLiveInterval()
  const [sort, setSort] = useState<TemplateSort>({ key: 'count', desc: true })
  const services = useQuery(api.services())
  const templates = useQuery({
    ...api.logTemplates({ since, service: search.service, q: search.q }),
    refetchInterval,
    // Keep the old rows while a new filter loads, instead of flashing skeletons.
    placeholderData: keepPreviousData,
  })

  const onSearch = useCallback(
    (patch: Partial<LogTemplatesSearch>) =>
      void navigate({ search: (prev) => ({ ...prev, ...patch }), replace: true, resetScroll: false }),
    [navigate],
  )
  const onQ = useCallback((q: string | undefined) => onSearch({ q }), [onSearch])
  const [text, setText] = useDebouncedParam(search.q, onQ, SEARCH_DEBOUNCE_MS)

  const filtered = Boolean(search.service || search.q)
  const data = templates.data
  const clear = () => {
    setText('')
    onSearch({ service: undefined, q: undefined })
  }

  return (
    <div className="flex flex-col gap-3.5">
      <Card className="overflow-hidden" aria-busy={templates.isFetching || undefined}>
        <div className="flex flex-wrap items-center gap-2 border-b border-line px-4 py-2.5 text-muted">
          <Combobox
            label="Service"
            value={search.service}
            options={services.data ?? []}
            onChange={(service) => onSearch({ service })}
            emptyText="No services"
          />
          <label className="flex h-[30px] min-w-[170px] flex-1 items-center gap-1.5 rounded-control border border-field-line bg-field px-2 shadow-inset sm:max-w-[320px]">
            <Search aria-hidden size={13} className="shrink-0" />
            <input
              type="search"
              aria-label="Search templates"
              placeholder="Search templates"
              maxLength={200}
              value={text}
              onChange={(e) => setText(e.target.value)}
              className="min-w-0 flex-1 border-0 bg-transparent font-[inherit] text-[13px] text-ink outline-none placeholder:text-faint"
            />
          </label>
          {filtered ? (
            <Button size="sm" variant="ghost" onClick={clear}>
              Clear filters
            </Button>
          ) : null}
          <span className="ml-auto text-xs" aria-live="polite">
            {data ? `${data.length}${data.length >= API_LIMIT ? '+' : ''} ${data.length === 1 ? 'template' : 'templates'}` : null}
          </span>
        </div>
        {templates.isPending ? (
          <TableSkeleton />
        ) : loadFailed(templates) ? (
          <div className="p-4">
            <ErrorBanner what="log templates" error={templates.error} onRetry={() => void templates.refetch()} />
          </div>
        ) : data && data.length === 0 ? (
          <EmptyState
            title={filtered ? 'No templates match these filters' : 'No log templates in this window'}
            description={
              filtered ? 'Clear the search or the service filter to see more.' : `No logs arrived in the last ${since}. A longer time range may show older templates.`
            }
            action={
              filtered ? (
                <Button size="sm" onClick={clear}>
                  Clear filters
                </Button>
              ) : null
            }
          />
        ) : data ? (
          <Reveal>
            <TemplatesTable rows={data} since={since} nowMs={templates.dataUpdatedAt} sort={sort} onSort={setSort} />
          </Reveal>
        ) : null}
      </Card>
      <RefreshNote queries={[templates]} />
    </div>
  )
}
