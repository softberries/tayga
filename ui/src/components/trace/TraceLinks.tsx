import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { ExternalLink, SearchX } from 'lucide-react'
import { api } from '../../api/queries'
import { rangeSearch } from '../../app/range'
import { useRange } from '../../app/useRange'
import { Button } from '../ui/Button'
import { Card } from '../ui/Card'
import { EmptyState } from '../ui/EmptyState'

/** `{jaeger_url}/trace/{id}` when the API reports an http(s) Jaeger URL, else null. */
export function jaegerTraceUrl(base: string | null | undefined, traceId: string): string | null {
  if (!base || !/^https?:\/\//i.test(base)) return null
  return `${base.replace(/\/+$/, '')}/trace/${encodeURIComponent(traceId)}`
}

export function JaegerLink({ traceId }: { traceId: string }) {
  const config = useQuery(api.config())
  const href = jaegerTraceUrl(config.data?.jaeger_url, traceId)
  if (!href) return null
  return (
    <Button asChild size="sm" variant="secondary">
      <a href={href} target="_blank" rel="noreferrer noopener">
        <ExternalLink aria-hidden size={14} /> Open in Jaeger
      </a>
    </Button>
  )
}

export function NotFoundCard({ title, description }: { title: string; description: string }) {
  const range = useRange()
  return (
    <Card className="tg-in">
      <EmptyState
        icon={<SearchX size={18} />}
        title={title}
        description={description}
        action={
          <Button asChild size="sm">
            <Link to="/" search={rangeSearch(range)}>
              Go to stories
            </Link>
          </Button>
        }
      />
    </Card>
  )
}
