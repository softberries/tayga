import { Skeleton } from '../ui/Skeleton'

/**
 * Shown while the session guard waits on a slow API (after the router's pending delay): the
 * shell's outline with placeholders, so the page is never blank. Static: no queries, no links.
 */
export function AppPending() {
  return (
    <div role="status" aria-label="Loading Tayga" className="flex min-h-dvh bg-ground text-ink max-sm:flex-col-reverse">
      <div className="flex w-[68px] shrink-0 flex-col items-center gap-3 border-r border-line bg-rail py-4 max-sm:h-14 max-sm:w-full max-sm:flex-row max-sm:justify-around max-sm:border-r-0 max-sm:border-t max-sm:py-0">
        <img src="/logo-mark.png" alt="" width={34} height={34} className="mb-3.5 size-[34px] rounded-field object-cover shadow-brand max-sm:hidden" />
        {[0, 1, 2, 3, 4].map((i) => (
          <Skeleton key={i} className="size-11 rounded-rail" />
        ))}
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex items-center gap-3 border-b border-line bg-header px-6 py-3.5">
          <Skeleton className="h-5 w-40" />
          <div className="flex-1" />
          <Skeleton className="h-9 w-48 rounded-field max-sm:hidden" />
          <Skeleton className="h-9 w-11 rounded-field" />
        </div>
        <div className="flex flex-col gap-4 px-6 py-[18px] max-sm:px-4">
          <Skeleton className="h-28 rounded-panel" />
          <Skeleton className="h-72 rounded-panel" />
        </div>
      </div>
    </div>
  )
}
