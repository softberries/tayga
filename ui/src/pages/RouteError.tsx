import { useQueryErrorResetBoundary } from '@tanstack/react-query'
import { useRouter } from '@tanstack/react-router'
import type { ErrorComponentProps } from '@tanstack/react-router'
import { useEffect } from 'react'
import { Card } from '../components/ui/Card'
import { ErrorState } from '../components/ui/ErrorState'

/** Root error boundary: any render or loader error inside the shell lands here. */
export function RouteError({ error, reset }: ErrorComponentProps) {
  const router = useRouter()
  const queryReset = useQueryErrorResetBoundary()
  useEffect(() => queryReset.reset(), [queryReset])
  return (
    <Card className="tg-in">
      <ErrorState
        error={error}
        onRetry={() => {
          reset()
          void router.invalidate()
        }}
      />
    </Card>
  )
}
