import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { LoaderCircle } from 'lucide-react'
import { m, useAnimationControls, useReducedMotionConfig } from 'motion/react'
import type { FormEvent } from 'react'
import { useEffect, useId, useRef, useState } from 'react'
import { isApiError } from '../api/client'
import { authApi } from '../api/queries'
import { safeNext } from '../app/auth'
import { ThemeSwitch } from '../components/shell/ThemeSwitch'
import { Button } from '../components/ui/Button'
import { Card } from '../components/ui/Card'

const fieldClass =
  'h-10 w-full rounded-field border border-field-line bg-field px-3 text-[13px] text-ink shadow-inset placeholder:text-faint ' +
  'transition-colors duration-150 hover:border-accent/60'

/** The inline error for a failed sign-in. */
function failureText(e: unknown): string {
  if (!isApiError(e)) return 'Sign-in failed. Try again.'
  if (e.status === 401) return 'Wrong username or password.'
  if (e.status === 429) {
    return e.retryAfter ? `Too many attempts, try again in ${e.retryAfter} s` : 'Too many attempts, try again later'
  }
  if (e.status === 0) return 'Cannot reach Tayga. Check the connection and try again.'
  return `Sign-in failed (HTTP ${e.status}). Try again.`
}

/** `/login`: outside the shell. Signing in resets every cached query and goes to a safe `next`. */
export function LoginPage() {
  const { next } = useSearch({ from: '/login' })
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const reduce = useReducedMotionConfig() ?? false
  const shake = useAnimationControls()
  const id = useId()
  const userRef = useRef<HTMLInputElement>(null)
  const passRef = useRef<HTMLInputElement>(null)
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)

  useEffect(() => userRef.current?.focus(), [])

  const login = useMutation({
    mutationFn: authApi.login,
    onSuccess: async () => {
      queryClient.clear()
      await navigate({ href: safeNext(next), replace: true })
    },
    onError: (e) => {
      setError(failureText(e))
      setPassword('')
      passRef.current?.focus()
      if (!reduce) void shake.start({ x: [0, -10, 9, -6, 4, -2, 0], transition: { duration: 0.42, ease: 'easeOut' } })
    },
  })

  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault()
    if (login.isPending) return
    setError(null)
    login.mutate({ username, password })
  }

  return (
    <div className="tg-login-ground relative flex min-h-dvh items-center justify-center overflow-hidden bg-ground px-4 py-16 text-ink">
      <div className="absolute right-4 top-4">
        <ThemeSwitch />
      </div>
      <main className="w-full max-w-[380px]">
        <m.div
          initial={reduce ? false : { opacity: 0, y: 14, scale: 0.98 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          transition={{ duration: 0.38, ease: [0.22, 1, 0.36, 1] }}
        >
          <m.div animate={shake}>
            <Card elevated className="px-8 pb-8 pt-9 max-sm:px-6">
              <div className="flex flex-col items-center text-center">
                <img src="/logo-mark.png" alt="" width={52} height={52} className="size-[52px] rounded-[14px] object-cover shadow-brand" />
                <h1 className="m-0 mt-4 text-[22px] font-semibold tracking-[-0.01em]">Tayga</h1>
                <p className="m-0 mt-1 text-muted">Sign in to continue</p>
              </div>

              <form className="mt-7 flex flex-col gap-4" onSubmit={submit} aria-describedby={error ? `${id}-error` : undefined}>
                <div className="flex flex-col gap-1.5">
                  <label htmlFor={`${id}-user`} className="text-xs font-medium text-ink-2">
                    Username
                  </label>
                  <input
                    ref={userRef}
                    id={`${id}-user`}
                    name="username"
                    autoComplete="username"
                    autoCapitalize="none"
                    spellCheck={false}
                    required
                    value={username}
                    onChange={(e) => setUsername(e.target.value)}
                    className={fieldClass}
                  />
                </div>
                <div className="flex flex-col gap-1.5">
                  <label htmlFor={`${id}-pass`} className="text-xs font-medium text-ink-2">
                    Password
                  </label>
                  <input
                    ref={passRef}
                    id={`${id}-pass`}
                    name="password"
                    type="password"
                    autoComplete="current-password"
                    required
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    aria-invalid={error ? true : undefined}
                    className={fieldClass}
                  />
                </div>

                {error ? (
                  <p
                    id={`${id}-error`}
                    role="alert"
                    className="m-0 rounded-field border border-err/40 bg-err-soft px-3 py-2 text-[13px] text-err"
                  >
                    {error}
                  </p>
                ) : null}

                <Button type="submit" variant="primary" className="mt-1 h-10 w-full" disabled={login.isPending} aria-busy={login.isPending || undefined}>
                  {login.isPending ? (
                    <>
                      <LoaderCircle size={15} className="motion-safe:animate-spin" aria-hidden />
                      Signing in…
                    </>
                  ) : (
                    'Sign in'
                  )}
                </Button>
              </form>
            </Card>
          </m.div>
        </m.div>
      </main>
    </div>
  )
}
