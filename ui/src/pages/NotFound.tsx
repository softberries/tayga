import { Link } from '@tanstack/react-router'
import { SearchX } from 'lucide-react'
import { sinceSearch } from '../app/search'
import { useSince } from '../components/shell/TimeRange'
import { Button } from '../components/ui/Button'
import { Card } from '../components/ui/Card'
import { EmptyState } from '../components/ui/EmptyState'

export function NotFound() {
  const since = useSince()
  return (
    <Card className="tg-in">
      <EmptyState
        icon={<SearchX size={18} />}
        title="Page not found"
        description="This address does not match any Tayga page."
        action={
          <Button asChild size="sm">
            <Link to="/" search={sinceSearch(since)}>
              Go to stories
            </Link>
          </Button>
        }
      />
    </Card>
  )
}
