import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { ChevronDown, LogOut, UserRound } from 'lucide-react'
import { signOut, useSession } from '../../app/auth'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from '../ui/DropdownMenu'

/** The signed-in user's chip and its menu; rendered only when auth is enabled. */
export function UserMenu() {
  const { authEnabled, username } = useSession()
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  if (!authEnabled || !username) return null
  return (
    <DropdownMenu>
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
        <DropdownMenuItem onSelect={() => void signOut(queryClient, navigate)}>
          <LogOut size={14} aria-hidden className="text-muted" />
          Sign out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
