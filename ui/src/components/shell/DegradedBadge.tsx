import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { api } from '../../api/queries'
import { useLiveInterval } from '../../app/live'
import { Badge } from '../ui/Badge'
import { useSince } from './TimeRange'

/** "N services degraded" from /service-map node health; hidden when all are ok or on error. */
export function DegradedBadge() {
  const since = useSince()
  const refetchInterval = useLiveInterval()
  const { data } = useQuery({ ...api.serviceMap(since), refetchInterval })
  if (!data) return null
  const bad = data.nodes.filter((n) => n.health !== 'ok')
  if (bad.length === 0) return null
  const errors = bad.some((n) => n.health === 'error')
  const names = bad.map((n) => n.service).join(', ')
  return (
    <Link to="/map" search={(prev) => prev} title={names} className="rounded-full">
      <Badge kind={errors ? 'error' : 'slow'} shape="pill" pulse>
        {bad.length} {bad.length === 1 ? 'service' : 'services'} degraded
      </Badge>
    </Link>
  )
}
