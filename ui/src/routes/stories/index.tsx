/**
 * `/` Stories home (the chosen design board): KPI tiles, the story-groups table with its
 * inspector, and a mini service map and log alerts below. Filters and the selected group live
 * in the URL; every query refreshes in live mode.
 */
import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate, useSearch } from '@tanstack/react-router'
import { useCallback, useEffect, useState } from 'react'
import { api } from '../../api/queries'
import type { StoryGroup } from '../../api/types'
import { doubledRange, rangeParams, rangePhrase, rangeSearch, widerHint } from '../../app/range'
import type { HomeSearch } from '../../app/search'
import { useAutoRefresh, useRange } from '../../app/useRange'
import { Card, PanelTitle } from '../../components/ui/Card'
import { EmptyState } from '../../components/ui/EmptyState'
import { Skeleton } from '../../components/ui/Skeleton'
import { RefreshNote, loadFailed } from '../../components/ui/StaleNote'
import { AlertsPanel } from '../../features/stories/AlertsPanel'
import { ErrorBanner } from '../../features/stories/ErrorBanner'
import { GroupsTable } from '../../features/stories/GroupsTable'
import { Inspector, WIDE_QUERY } from '../../features/stories/Inspector'
import { KpiTiles, KpiTilesSkeleton } from '../../features/stories/KpiTiles'
import { MiniMap } from '../../features/stories/MiniMap'
import { useMediaQuery } from '../../lib/useMediaQuery'
import { Reveal } from '../../components/ui/Reveal'

function TableSkeleton() {
  return (
    <div aria-busy="true" aria-label="Loading story groups">
      <div className="flex gap-2 border-b border-line px-4 py-2.5">
        <Skeleton className="h-[30px] w-24" />
        <Skeleton className="h-[30px] w-20" />
        <Skeleton className="h-[30px] w-24" />
      </div>
      {Array.from({ length: 6 }, (_, i) => (
        <div key={i} className="flex items-center gap-4 border-b border-line-soft px-4 py-3 last:border-b-0">
          <div className="flex flex-1 flex-col gap-1.5">
            <Skeleton className="h-4 w-3/5" />
            <Skeleton className="h-3 w-2/5" />
          </div>
          <Skeleton className="hidden h-7 w-[120px] sm:block" />
          <Skeleton className="h-4 w-10" />
        </div>
      ))}
    </div>
  )
}

export function StoriesHome() {
  const range = useRange()
  const search = useSearch({ from: '/' })
  const navigate = useNavigate({ from: '/' })
  const refetchInterval = useAutoRefresh()
  const wide = useMediaQuery(WIDE_QUERY)

  const win = rangeParams(range)
  const overview = useQuery({ ...api.overview(win), refetchInterval })
  // The current and the previous window together, for the "vs prev" deltas; null past 7 days
  // or retention. The clock only matters for a custom range, whose end is fixed.
  const [now] = useState(() => Date.now())
  const dbl = doubledRange(range, now)
  const doubled = useQuery({ ...api.overview(dbl ? rangeParams(dbl) : win), enabled: dbl !== null, refetchInterval })
  const groups = useQuery({ ...api.storyGroups(win), refetchInterval })
  const map = useQuery({ ...api.serviceMap(win), refetchInterval })
  const alerts = useQuery({ ...api.logAlerts(win), refetchInterval })

  const onSearch = useCallback(
    (patch: Partial<HomeSearch>) =>
      void navigate({ search: (prev) => ({ ...prev, ...patch }), replace: true, resetScroll: false }),
    [navigate],
  )
  const onOpen = useCallback(
    (g: StoryGroup) => void navigate({ to: '/stories/$storyId', params: { storyId: g.sample_story_id } }),
    [navigate],
  )
  const onSelect = useCallback((group: string) => onSearch({ group }), [onSearch])
  const [visible, setVisible] = useState<StoryGroup[]>([])
  // The URL's group when it is visible, else the top row.
  const selected = visible.find((g) => g.fingerprint === search.group) ?? visible[0]
  // A selected group that was filtered out or refreshed away leaves the URL, so the URL
  // always names what the inspector shows. An empty table keeps it (clearing filters restores it).
  const stale = search.group !== undefined && visible.length > 0 && !visible.some((g) => g.fingerprint === search.group)
  useEffect(() => {
    if (stale) onSearch({ group: undefined })
  }, [stale, onSearch])

  const empty = groups.data?.length === 0

  const table = (
    <Card className="overflow-hidden">
      {groups.isPending ? (
        <TableSkeleton />
      ) : loadFailed(groups) ? (
        <div className="p-4">
          <ErrorBanner what="story groups" error={groups.error} onRetry={() => void groups.refetch()} />
        </div>
      ) : empty ? (
        <EmptyState
          title="No stories in this window"
          description={`Nothing failed or ran slow in ${rangePhrase(range)}.${widerHint(range, 'stories')}`}
        />
      ) : groups.data ? (
        <Reveal>
          <RefreshNote queries={[groups]} className="mx-4 mt-3" />
          <GroupsTable
            groups={groups.data}
            search={search}
            range={range}
            nowMs={groups.dataUpdatedAt}
            onSearch={onSearch}
            selected={selected?.fingerprint}
            onSelect={onSelect}
            onOpen={onOpen}
            onVisible={setVisible}
          />
        </Reveal>
      ) : null}
    </Card>
  )

  const below = (
    <div className="grid gap-3.5 md:grid-cols-2">
      <Card lift={false} className="flex min-w-0 flex-col gap-1.5 px-4 py-3.5">
        <PanelTitle>Service map</PanelTitle>
        {map.isPending ? (
          <Skeleton className="h-[190px]" />
        ) : loadFailed(map) || !map.data ? (
          <ErrorBanner what="the service map" error={map.error} onRetry={() => void map.refetch()} />
        ) : (
          <Reveal>
            <RefreshNote queries={[map]} className="mb-1.5" />
            <MiniMap map={map.data} range={range} />
          </Reveal>
        )}
      </Card>
      <Card className="flex min-w-0 flex-col gap-2.5 px-4 py-3.5">
        <div className="flex items-baseline gap-2">
          <PanelTitle>Log alerts</PanelTitle>
          <Link to="/logs/alerts" search={rangeSearch(range)} className="ml-auto text-xs text-accent hover:underline">
            View all
          </Link>
        </div>
        {alerts.isPending ? (
          <div className="flex flex-col gap-2.5" aria-busy="true">
            <Skeleton className="h-[54px]" />
            <Skeleton className="h-[54px]" />
          </div>
        ) : loadFailed(alerts) || !alerts.data ? (
          <ErrorBanner what="log alerts" error={alerts.error} onRetry={() => void alerts.refetch()} />
        ) : (
          <Reveal>
            <RefreshNote queries={[alerts]} className="mb-2.5" />
            <AlertsPanel alerts={alerts.data} range={range} nowMs={alerts.dataUpdatedAt} />
          </Reveal>
        )}
      </Card>
    </div>
  )

  const showInspector = !empty && !loadFailed(groups)
  return (
    <div className="flex flex-col gap-[18px]">
      {overview.isPending ? (
        <KpiTilesSkeleton />
      ) : loadFailed(overview) || !overview.data ? (
        <ErrorBanner what="the summary" error={overview.error} onRetry={() => void overview.refetch()} />
      ) : (
        <>
          <RefreshNote queries={[overview, doubled]} />
          <KpiTiles overview={overview.data} doubled={doubled} alerts={alerts.data} range={range} nowMs={overview.dataUpdatedAt} />
        </>
      )}
      {wide ? (
        <div className="flex items-start gap-3.5">
          <section aria-label="Story groups" className="flex min-w-0 flex-1 flex-col gap-3.5">
            {table}
            {below}
          </section>
          {showInspector ? <Inspector group={selected} pending={groups.isPending} /> : null}
        </div>
      ) : (
        <>
          <section aria-label="Story groups">{table}</section>
          {showInspector ? <Inspector group={selected} pending={groups.isPending} /> : null}
          {below}
        </>
      )}
    </div>
  )
}
