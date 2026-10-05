/**
 * Service drawer on the map (`/map?service=`): RED tiles with small-multiple charts, story
 * groups whose root cause is the service, its log signals, callers and callees, and a link to
 * the traces explorer filtered to it.
 */
import { useQuery } from '@tanstack/react-query'
import type { UseQueryResult } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { ArrowRight } from 'lucide-react'
import { useMemo } from 'react'
import type { ReactNode } from 'react'
import { api } from '../../api/queries'
import type { EdgeView, Health, LogAlertView, LogTemplateView, NodeView, ServiceMapView, ServiceView, StoryGroup } from '../../api/types'
import { useLiveInterval } from '../../app/live'
import { sinceSearch } from '../../app/search'
import type { Since } from '../../app/search'
import { TimeSeries } from '../../components/charts/TimeSeries'
import type { SeriesTone } from '../../components/charts/TimeSeries'
import { Badge } from '../../components/ui/Badge'
import { Button } from '../../components/ui/Button'
import { ErrorState } from '../../components/ui/ErrorState'
import { Sheet } from '../../components/ui/Sheet'
import { Skeleton } from '../../components/ui/Skeleton'
import { cx } from '../../lib/cx'
import { ago, compact, duration, percent } from '../../lib/format'
import { endpointOf, SINCE_SECS, splitSummary } from '../stories/model'
import { isFailingEdge } from '../stories/mapLayout'
import { HealthRing } from './ServiceNode'
import { errText, neighbours, nodeOf, rateText } from './model'
import { TruncationTooltip } from '../../components/ui/Tooltip'

export const DRAWER_KEY = 'tayga-map-drawer-width'
const GROUPS_SHOWN = 5
const SIGNALS_SHOWN = 5

/** Matches the API's rule: error at 5 % failed calls, slow above twice the 24 h p99. */
export function healthText(n: NodeView): string {
  if (n.health === 'error') return `Failing: ${percent(n.error_ratio)} of calls failed`
  if (n.health === 'slow') return `Slow: p99 ${duration(n.p99_ns)} vs ${duration(n.baseline_p99_ns)} over 24 h`
  return 'Healthy'
}

function Section({ title, children, action }: { title: string; children: ReactNode; action?: ReactNode }) {
  return (
    <section aria-label={title} className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <h3 className="m-0 text-xs font-medium uppercase tracking-[0.06em] text-muted">{title}</h3>
        {action ? <span className="ml-auto">{action}</span> : null}
      </div>
      {children}
    </section>
  )
}

const Muted = ({ children }: { children: ReactNode }) => <p className="m-0 text-muted">{children}</p>

function ListSkeleton({ rows = 2 }: { rows?: number }) {
  return (
    <div aria-busy="true" className="flex flex-col gap-2">
      {Array.from({ length: rows }, (_, i) => (
        <Skeleton key={i} className="h-12 w-full" />
      ))}
    </div>
  )
}

interface RedSpec {
  label: string
  value: string
  tone: SeriesTone
  points: [number, number][]
  format: (v: number) => string
  summary: string
}

const rateFmt = rateText
const pctFmt = (v: number) => `${v < 10 && v !== 0 ? v.toFixed(1) : Math.round(v)} %`
const msFmt = (v: number) => duration(v * 1e6)

function range(points: readonly [number, number][], fmt: (v: number) => string): string {
  if (points.length === 0) return 'no data'
  let lo = Infinity
  let hi = -Infinity
  for (const [, v] of points) {
    lo = Math.min(lo, v)
    hi = Math.max(hi, v)
  }
  return `from ${fmt(lo)} to ${fmt(hi)}`
}

export function redSpecs(service: string, view: ServiceView, node: NodeView | undefined, since: Since): RedSpec[] {
  const at = (b: { bucket: number }) => b.bucket * 1000
  const rate = view.buckets.map((b) => [at(b), b.rate] as [number, number])
  const errors = view.buckets.map((b) => [at(b), b.error_ratio * 100] as [number, number])
  const p99 = view.buckets.map((b) => [at(b), b.p99_ns / 1e6] as [number, number])
  const windowRate = view.calls / SINCE_SECS[since]
  const errRatio = node?.error_ratio ?? (view.calls > 0 ? view.errors / view.calls : 0)
  const per = `per ${view.bucket_secs} s bucket over the last ${since}`
  return [
    {
      label: 'Rate',
      value: rateFmt(node?.rate ?? windowRate),
      tone: 'accent',
      points: rate,
      format: rateFmt,
      summary: `${service} calls per second ${per}: ${range(rate, rateFmt)}.`,
    },
    {
      label: 'Errors',
      value: errText(errRatio),
      tone: 'err',
      points: errors,
      format: pctFmt,
      summary: `${service} error percentage ${per}: ${range(errors, pctFmt)}.`,
    },
    {
      label: 'p99',
      value: node ? duration(node.p99_ns) : '—',
      tone: node?.health === 'slow' ? 'slow' : 'accent',
      points: p99,
      format: msFmt,
      summary: `${service} p99 latency ${per}: ${range(p99, msFmt)}.`,
    },
  ]
}

function RedTiles({ service, red, node, since }: { service: string; red: UseQueryResult<ServiceView>; node: NodeView | undefined; since: Since }) {
  const specs = useMemo(() => (red.data ? redSpecs(service, red.data, node, since) : null), [red.data, service, node, since])
  if (red.isPending) {
    return (
      <div aria-busy="true" aria-label="Loading RED metrics" className="flex flex-col gap-2">
        {[0, 1, 2].map((i) => (
          <Skeleton key={i} className="h-[118px] w-full" />
        ))}
      </div>
    )
  }
  if (!specs) return <ErrorState error={red.error} onRetry={() => void red.refetch()} className="py-4" />
  if (red.data?.buckets.length === 0) return <Muted>No calls to {service} in the last {since}.</Muted>
  return (
    <ul className="m-0 flex list-none flex-col gap-2 p-0">
      {specs.map((s) => (
        <li key={s.label} className="flex flex-col gap-1 rounded-field border border-panel-line bg-inner px-3 pb-1 pt-2.5">
          <div className="flex items-baseline gap-2">
            <span className="text-[11px] text-muted">{s.label}</span>
            <span
              className={cx(
                'tabular ml-auto text-[18px] font-semibold',
                s.label === 'Errors' && s.value !== '0 %' && 'text-err',
                s.tone === 'slow' && 'text-slow',
              )}
            >
              {s.value}
            </span>
          </div>
          <TimeSeries
            series={[{ name: s.label, points: s.points, tone: s.tone, type: 'area' }]}
            height={92}
            format={s.format}
            minInterval={0}
            splitNumber={2}
            summary={s.summary}
          />
        </li>
      ))}
    </ul>
  )
}

function Stories({ groups, since, service, nowMs }: { groups: readonly StoryGroup[]; since: Since; service: string; nowMs: number }) {
  if (groups.length === 0) return <Muted>No stories have their root cause in {service}.</Muted>
  const top = [...groups].sort((a, b) => b.stories - a.stories).slice(0, GROUPS_SHOWN)
  return (
    <ul className="m-0 flex list-none flex-col gap-2 p-0">
      {top.map((g) => (
        <li key={g.fingerprint}>
          <Link
            to="/"
            search={{ ...sinceSearch(since), service, group: g.fingerprint }}
            className={cx(
              'tg-card flex flex-col gap-1.5 rounded-field border px-3 py-2.5',
              g.kind === 'error' ? 'border-err/35 bg-err-soft' : 'border-slow/35 bg-slow-soft',
            )}
          >
            <span className="flex min-w-0 items-center gap-2">
              <Badge kind={g.kind === 'error' ? 'error' : 'slow'}>{g.kind}</Badge>
              <TruncationTooltip content={g.summary} openOnHostFocus>
                <span className="min-w-0 truncate font-medium text-ink">{splitSummary(g.summary).title}</span>
              </TruncationTooltip>
            </span>
            <span className="tabular truncate font-mono text-[11.5px] text-muted">
              {compact(g.stories)} {g.stories === 1 ? 'story' : 'stories'} · {endpointOf(g)} · {ago(g.last_seen_ns, nowMs)}
            </span>
          </Link>
        </li>
      ))}
    </ul>
  )
}

type Signal = { kind: 'new' | 'spike' | 'alerting'; templateId: string; template: string; detail: string; active: boolean }

/**
 * One signal per template: its latest alert (active ones first), then templates flagged as
 * alerting without an alert in the list.
 */
export function logSignals(alerts: readonly LogAlertView[], templates: readonly LogTemplateView[]): Signal[] {
  const out: Signal[] = []
  const seen = new Set<string>()
  const sorted = [...alerts].sort((a, b) => Number(b.active) - Number(a.active) || b.last_at_ns - a.last_at_ns)
  for (const a of sorted) {
    if (seen.has(a.template_id)) continue
    seen.add(a.template_id)
    out.push({
      kind: a.kind,
      templateId: a.template_id,
      template: a.template,
      active: a.active,
      detail: a.kind === 'new' ? `${a.window_count} in its first window` : `${a.peak_count} vs ${a.baseline_per_window.toFixed(1)} / window`,
    })
  }
  for (const t of templates) {
    if (!t.alerting || seen.has(t.template_id)) continue
    out.push({ kind: 'alerting', templateId: t.template_id, template: t.template, active: true, detail: `${compact(t.count)} lines` })
  }
  return out
}

function Signals({ signals, since, service }: { signals: readonly Signal[]; since: Since; service: string }) {
  if (signals.length === 0) return <Muted>No log alerts for {service} in this window.</Muted>
  return (
    <ul className="m-0 flex list-none flex-col gap-2 p-0">
      {signals.slice(0, SIGNALS_SHOWN).map((s) => (
        <li key={s.templateId}>
          <Link
            to="/logs/templates/$templateId"
            params={{ templateId: s.templateId }}
            search={sinceSearch(since)}
            className="tg-card flex flex-col gap-1.5 rounded-field border border-panel-line bg-inner px-3 py-2.5 hover:border-field-line"
          >
            <span className="flex min-w-0 items-center gap-2">
              <Badge kind={s.kind === 'alerting' ? 'error' : s.kind}>{s.kind}</Badge>
              {s.active ? <span className="sr-only">active</span> : null}
              {s.active ? <span aria-hidden className="tg-pulse size-1.5 shrink-0 rounded-full bg-err" /> : null}
              <span className="tabular ml-auto shrink-0 text-xs text-muted">{s.detail}</span>
            </span>
            <span className="line-clamp-2 break-all font-mono text-xs text-ink" title={s.template}>
              {s.template}
            </span>
          </Link>
        </li>
      ))}
    </ul>
  )
}

function Calls({ edges, side, since }: { edges: readonly EdgeView[]; side: 'callers' | 'callees'; since: Since }) {
  if (edges.length === 0) return <Muted>{side === 'callers' ? 'No callers in this window.' : 'Calls no other service.'}</Muted>
  return (
    <ul className="m-0 flex list-none flex-col p-0">
      {edges.map((e) => {
        const other = side === 'callers' ? e.parent : e.child
        const failing = isFailingEdge(e)
        return (
          <li key={`${e.parent}>${e.child}`} className="border-b border-line-soft last:border-b-0">
            <Link
              to="/map"
              search={{ ...sinceSearch(since), service: other }}
              className="flex min-w-0 items-center gap-3 py-1.5 font-mono text-xs hover:text-accent"
            >
              <span className="min-w-0 truncate">{side === 'callers' ? `${other} →` : `→ ${other}`}</span>
              <span className={cx('tabular ml-auto shrink-0', failing ? 'text-err' : 'text-muted')}>
                {compact(e.calls)} calls · {errText(e.error_rate)} err · {duration(e.avg_duration_ns)}
              </span>
            </Link>
          </li>
        )
      })}
    </ul>
  )
}

function DrawerBody({ service, map, since }: { service: string; map: ServiceMapView | undefined; since: Since }) {
  const refetchInterval = useLiveInterval()
  const red = useQuery({ ...api.service(service, since), refetchInterval })
  const groups = useQuery({ ...api.storyGroups({ since, service }), refetchInterval })
  const alerts = useQuery({ ...api.logAlerts({ since, service }), refetchInterval })
  const templates = useQuery({ ...api.logTemplates({ since, service }), refetchInterval })
  const node = map ? nodeOf(map, service) : undefined
  const near = useMemo(() => (map ? neighbours(map, service) : { callers: [], callees: [] }), [map, service])
  const signals = useMemo(() => logSignals(alerts.data ?? [], templates.data ?? []), [alerts.data, templates.data])
  // "Last seen" is relative to when the groups arrived (Stories renders only with data).
  const nowMs = groups.dataUpdatedAt

  return (
    <div className="flex flex-col gap-5">
      <Button asChild variant="primary" size="md" className="w-full">
        <Link to="/traces" search={{ ...sinceSearch(since), service }}>
          Open {service} traces <ArrowRight aria-hidden size={14} />
        </Link>
      </Button>

      <Section title={`RED · last ${since}`}>
        <RedTiles service={service} red={red} node={node} since={since} />
      </Section>

      <Section title="Stories with root cause here">
        {groups.isPending ? (
          <ListSkeleton />
        ) : groups.isError ? (
          <ErrorState error={groups.error} onRetry={() => void groups.refetch()} className="py-4" />
        ) : (
          <Stories groups={groups.data} since={since} service={service} nowMs={nowMs} />
        )}
      </Section>

      <Section title="Log signals">
        {alerts.isPending || templates.isPending ? (
          <ListSkeleton />
        ) : alerts.isError || templates.isError ? (
          <ErrorState
            error={alerts.error ?? templates.error}
            onRetry={() => {
              void alerts.refetch()
              void templates.refetch()
            }}
            className="py-4"
          />
        ) : (
          <Signals signals={signals} since={since} service={service} />
        )}
      </Section>

      <Section title="Callers">
        <Calls edges={near.callers} side="callers" since={since} />
      </Section>
      <Section title="Callees">
        <Calls edges={near.callees} side="callees" since={since} />
      </Section>
    </div>
  )
}

export interface ServiceDrawerProps {
  /** The open service, or undefined when closed. */
  service: string | undefined
  map: ServiceMapView | undefined
  since: Since
  onClose: () => void
  onCloseAutoFocus?: (e: Event) => void
}

export function ServiceDrawer({ service, map, since, onClose, onCloseAutoFocus }: ServiceDrawerProps) {
  const node = service && map ? nodeOf(map, service) : undefined
  const onMap = Boolean(service && map && (node || map.edges.some((e) => e.parent === service || e.child === service)))
  const health: Health = node?.health ?? 'ok'
  return (
    <Sheet
      open={service !== undefined}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      storageKey={DRAWER_KEY}
      className="tg-service-drawer"
      onCloseAutoFocus={onCloseAutoFocus}
      // Focus the panel itself (its title is announced), not the resize handle.
      onOpenAutoFocus={(e) => {
        e.preventDefault()
        if (e.target instanceof HTMLElement) e.target.focus()
      }}
      title={
        <span className="flex min-w-0 items-center gap-2.5">
          <HealthRing health={health} errorRatio={node?.error_ratio ?? 0} size={22} />
          <span className="truncate">{service}</span>
        </span>
      }
      subtitle={
        !map ? undefined : !onMap ? (
          `Not on the map in the last ${since}`
        ) : node ? (
          <span className={cx(health === 'error' && 'text-err', health === 'slow' && 'text-slow')}>{healthText(node)}</span>
        ) : (
          'Caller only: no server spans of its own'
        )
      }
    >
      {service ? <DrawerBody key={service} service={service} map={map} since={since} /> : null}
    </Sheet>
  )
}
