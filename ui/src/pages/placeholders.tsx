/** Route components loaded lazily per route; each is replaced by its own task. */
import { Placeholder } from './Placeholder'

export function TracesPage() {
  return <Placeholder title="Traces explorer" task="Task 8" />
}
export function MapPage() {
  return <Placeholder title="Service map" task="Task 9" />
}
export function LogAlertsPage() {
  return <Placeholder title="Log alerts" task="Task 10" />
}
export function LogTemplatesPage() {
  return <Placeholder title="Log templates" task="Task 10" />
}
export function LogTemplatePage() {
  return <Placeholder title="Log template" task="Task 10" />
}
export function PipelinePage() {
  return <Placeholder title="Pipeline health" task="Task 11" />
}
