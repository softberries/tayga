import { useNavigate } from '@tanstack/react-router'
import { isApiError } from '../../api/client'
import { Button } from './Button'

/**
 * The API's 400 for a window that starts before its data retention: a custom range (`until`)
 * kept in a link or bookmark that has since aged past 7 days.
 */
export function isExpiredRange(error: unknown): boolean {
  return isApiError(error) && error.status === 400 && error.message.includes('data retention')
}

function ResetButton() {
  const navigate = useNavigate()
  return (
    <Button
      size="sm"
      variant="primary"
      onClick={() => void navigate({ to: '.', search: (prev: Record<string, unknown>) => ({ ...prev, since: undefined, until: undefined }) } as never)}
    >
      Show last 1h
    </Button>
  )
}

/**
 * "Show last 1h" for an expired custom range: it clears `until` and sets the default `since`
 * when clicked, so the URL never changes on its own. Nothing for any other error.
 */
export function ExpiredRangeReset({ error }: { error: unknown }) {
  return isExpiredRange(error) ? <ResetButton /> : null
}
