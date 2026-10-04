import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { api } from '../../api/queries'
import type { NodeView } from '../../api/types'
import { useLiveInterval } from '../../app/live'
import { sinceSearch } from '../../app/search'
import { Badge } from '../ui/Badge'
import { useSince } from './TimeRange'

/** Nodes from a /service-map response; an older API without `nodes` yields none. */
function nodesOf(data: unknown): NodeView[] {
  if (data && typeof data === 'object' && 'nodes' in data && Array.isArray(data.nodes)) {
    return data.nodes as NodeView[]
  }
  return []
}

/** "N services degraded" from /service-map node health; hidden when all are ok or on error. */
export function DegradedBadge() {
  const since = useSince()
  const refetchInterval = useLiveInterval()
  const { data } = useQuery({ ...api.serviceMap(since), refetchInterval })
  const bad = nodesOf(data).filter((n) => n.health !== 'ok')
  if (bad.length === 0) return null
  const errors = bad.some((n) => n.health === 'error')
  const text = `${bad.length} ${bad.length === 1 ? 'service' : 'services'} degraded`
  const names = bad.map((n) => n.service).join(', ')
  return (
    <Link to="/map" search={sinceSearch(since)} aria-label={`${text}: ${names}`} title={names} className="rounded-full">
      <Badge kind={errors ? 'error' : 'slow'} shape="pill" pulse>
        {text}
      </Badge>
    </Link>
  )
}
