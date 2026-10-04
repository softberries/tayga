import { Card } from '../components/ui/Card'
import { EmptyState } from '../components/ui/EmptyState'

/** Stand-in for pages that later tasks build. */
export function Placeholder({ title, task }: { title: string; task: string }) {
  return (
    <Card className="tg-in">
      <EmptyState title={title} description={`This page is built in ${task}.`} />
    </Card>
  )
}
