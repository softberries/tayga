import { TriangleAlert } from 'lucide-react'
import { useOutage } from '../../app/apiStatus'

/** Global banner while the API answers 503 (ClickHouse or Kafka down). */
export function OutageBanner() {
  const outage = useOutage()
  if (!outage) return null
  return (
    <div role="alert" className="mx-6 mt-3 flex items-center gap-3 rounded-field border border-err bg-err-soft px-4 py-2.5 text-err">
      <TriangleAlert size={16} aria-hidden />
      <span className="font-semibold">Storage unavailable.</span>
      <span className="min-w-0 truncate font-mono text-xs">{outage.message}</span>
      <span className="ml-auto text-xs">Retrying on the next refresh.</span>
    </div>
  )
}
