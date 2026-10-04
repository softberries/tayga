/** Route components loaded lazily per route; each is replaced by its own task. */
import { Placeholder } from './Placeholder'

export function PipelinePage() {
  return <Placeholder title="Pipeline health" task="Task 11" />
}
