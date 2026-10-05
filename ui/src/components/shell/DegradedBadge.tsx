import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { api } from '../../api/queries'
import type { NodeView } from '../../api/types'
import { useLiveInterval } from '../../app/live'
import { Badge } from '../ui/Badge'

/**
 * The badge's window: always the last 15 minutes, live. It tells what is degraded now, whatever
 * range the page shows, so a custom (past) range never leaves a stale "now" in the header.
 */
const BADGE_SINCE = '15m'

/** Nodes from a /service-map response; an older API without `nodes` yields none. */
function nodesOf(data: unknown): NodeView[] {
  if (data && typeof data === 'object' && 'nodes' in data && Array.isArray(data.nodes)) {
    return data.nodes as NodeView[]
  }
  return []
}

/** "N services degraded" from /service-map node health over the last 15 min; hidden when all are ok or on error. */
export function DegradedBadge() {
  // Live even during a custom range: this is the current state.
  const refetchInterval = useLiveInterval()
  const { data } = useQuery({ ...api.serviceMap({ since: BADGE_SINCE }), refetchInterval })
  const bad = nodesOf(data).filter((n) => n.health !== 'ok')
  if (bad.length === 0) return null
  const errors = bad.some((n) => n.health === 'error')
  const text = `${bad.length} ${bad.length === 1 ? 'service' : 'services'} degraded`
  const names = bad.map((n) => n.service).join(', ')
  return (
    <Link
      to="/map"
      search={{ since: BADGE_SINCE, until: undefined }}
      aria-label={`${text} in the last 15 minutes: ${names}`}
      title={`Last 15 minutes: ${names}`}
      className="rounded-full"
    >
      <Badge kind={errors ? 'error' : 'slow'} shape="pill" pulse>
        {text}
      </Badge>
    </Link>
  )
}
