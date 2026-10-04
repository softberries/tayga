import { useEffect, useState } from 'react'

/** The current time in unix ms, re-read every `everyMs`, so ages advance without any fetch. */
export function useNow(everyMs = 15_000): number {
  const [now, setNow] = useState(Date.now)
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), everyMs)
    return () => window.clearInterval(id)
  }, [everyMs])
  return now
}
