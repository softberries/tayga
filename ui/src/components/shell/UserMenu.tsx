import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { ChevronDown, LogOut, UserRound } from 'lucide-react'
import { useState } from 'react'
import { SIGN_OUT_FAILED, signOut, useSession } from '../../app/auth'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from '../ui/DropdownMenu'

/** The signed-in user's chip and its menu; rendered only when auth is enabled. */
export function UserMenu() {
  const { authEnabled, username } = useSession()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [failed, setFailed] = useState(false)
  if (!authEnabled || !username) return null
  return (
    <DropdownMenu onOpenChange={() => setFailed(false)}>
      <DropdownMenuTrigger
        aria-label={`Signed in as ${username}`}
        className="inline-flex h-9 max-w-44 cursor-pointer items-center gap-2 rounded-field border border-field-line bg-field pl-1.5 pr-2.5 text-ink transition-colors duration-150 hover:bg-rail-active"
      >
        <span aria-hidden className="flex size-6 shrink-0 items-center justify-center rounded-full bg-rail-active text-accent">
          <UserRound size={14} strokeWidth={2} />
        </span>
        <span className="truncate text-[13px] max-sm:hidden">{username}</span>
        <ChevronDown size={14} aria-hidden className="shrink-0 text-muted" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel>Signed in as {username}</DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onSelect={(e) => {
            // Stays open: on success the page leaves for /login; on failure it says so here.
            e.preventDefault()
            setFailed(false)
            signOut(queryClient, navigate).catch(() => setFailed(true))
          }}
        >
          <LogOut size={14} aria-hidden className="text-muted" />
          Sign out
        </DropdownMenuItem>
        {failed ? (
          <p role="alert" className="m-0 max-w-52 px-2 pb-1.5 pt-1 text-xs text-err">
            {SIGN_OUT_FAILED}
          </p>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
