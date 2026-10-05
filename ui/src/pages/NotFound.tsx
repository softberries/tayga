import { Link } from '@tanstack/react-router'
import { SearchX } from 'lucide-react'
import { rangeSearch } from '../app/range'
import { useRange } from '../app/useRange'
import { Button } from '../components/ui/Button'
import { Card } from '../components/ui/Card'
import { EmptyState } from '../components/ui/EmptyState'

export function NotFound() {
  const range = useRange()
  return (
    <Card className="tg-in">
      <EmptyState
        icon={<SearchX size={18} />}
        title="Page not found"
        description="This address does not match any Tayga page."
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
