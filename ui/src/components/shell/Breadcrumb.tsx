import { useMatches } from '@tanstack/react-router'
import { Fragment } from 'react'
import { shortId } from '../../lib/format'

/** Crumbs from each matched route's `staticData.crumb`; the last one is the page title (h1). */
export function Breadcrumb() {
  const crumbs = useMatches({
    select: (matches) =>
      matches.flatMap((m) => {
        const crumb = m.staticData.crumb
        if (!crumb) return []
        const key = m.staticData.crumbParam
        const params = m.params as Record<string, string | undefined>
        const value = key ? params[key] : undefined
        return [value ? `${crumb} ${shortId(value)}` : crumb]
      }),
  })
  if (crumbs.length === 0) return null
  return (
    <nav aria-label="Breadcrumb" className="flex min-w-0 items-center gap-3">
      {crumbs.map((c, i) => (
        <Fragment key={i}>
          {i > 0 ? (
            <span aria-hidden className="text-faint">
              /
            </span>
          ) : null}
          {i === crumbs.length - 1 ? (
            <h1 className="m-0 truncate text-[15px] font-semibold">{c}</h1>
          ) : (
            <span className="truncate text-muted">{c}</span>
          )}
        </Fragment>
      ))}
    </nav>
  )
}
