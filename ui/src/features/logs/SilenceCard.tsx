/**
 * "Alert when silent" for one template: a switch and a threshold in minutes, saved with one PUT.
 * The minutes field is off while the switch is. A 401 follows the session-lost flow (the
 * mutation opts in); any other failure shows inline.
 */
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { BellRing } from 'lucide-react'
import { useId, useState } from 'react'
import { api } from '../../api/queries'
import type { LogTemplateDetail, SilenceSetting } from '../../api/types'
import { Button } from '../../components/ui/Button'
import { Card, PanelTitle } from '../../components/ui/Card'
import { Switch } from '../../components/ui/Switch'
import { cx } from '../../lib/cx'

export const SILENCE_MIN = 1
export const SILENCE_MAX = 1440
const DEFAULT_MINUTES = 10

interface Draft {
  enabled: boolean
  /** The field's text, so a half-typed value is not rewritten under the cursor. */
  minutes: string
}

/** The minutes as a whole number in 1..=1440, or undefined. */
export function parseMinutes(text: string): number | undefined {
  if (!/^\d{1,5}$/.test(text.trim())) return undefined
  const n = Number(text)
  return n >= SILENCE_MIN && n <= SILENCE_MAX ? n : undefined
}

export function SilenceCard({ templateId, silence }: { templateId: string; silence: SilenceSetting | null }) {
  const queryClient = useQueryClient()
  const id = useId()
  const saved: SilenceSetting = silence ?? { enabled: false, minutes: DEFAULT_MINUTES }
  const [draft, setDraft] = useState<Draft | null>(null)
  const [justSaved, setJustSaved] = useState(false)
  const cur: Draft = draft ?? { enabled: saved.enabled, minutes: String(saved.minutes) }
  const parsed = parseMinutes(cur.minutes)
  // A switched-off setting still stores minutes: keep the saved ones when the field is unusable.
  const minutes = parsed ?? saved.minutes
  const invalid = cur.enabled && parsed === undefined
  const dirty = cur.enabled !== saved.enabled || (cur.enabled && minutes !== saved.minutes)

  const save = useMutation({
    mutationFn: (s: SilenceSetting) => api.putSilence(templateId, s),
    meta: { sessionAware: true },
    onSuccess: (s) => {
      // Show the stored state at once; the invalidation below confirms it from the API.
      queryClient.setQueriesData<LogTemplateDetail>({ queryKey: ['log-templates', `/log-templates/${encodeURIComponent(templateId)}`] }, (d) =>
        d ? { ...d, silence: s } : d,
      )
      queryClient.setQueriesData<Array<{ template_id: string; silence_enabled: boolean }>>({ queryKey: ['log-templates', '/log-templates'] }, (rows) =>
        rows?.map((r) => (r.template_id === templateId ? { ...r, silence_enabled: s.enabled } : r)),
      )
      setDraft(null)
      setJustSaved(true)
      void queryClient.invalidateQueries({ queryKey: ['log-templates'] })
    },
  })

  const edit = (next: Partial<Draft>) => {
    setDraft({ ...cur, ...next })
    setJustSaved(false)
    save.reset()
  }

  const error = invalid ? `Minutes must be a whole number from ${SILENCE_MIN} to ${SILENCE_MAX}.` : save.isError ? save.error.message : undefined

  return (
    <Card className="flex flex-col gap-3 px-5 py-4">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <BellRing aria-hidden size={15} className="text-silence" />
        <PanelTitle>Silence alert</PanelTitle>
        <span className="text-xs text-muted">raise an alert when no log matches this template for a while</span>
      </div>
      <form
        className="flex flex-wrap items-center gap-x-5 gap-y-3"
        onSubmit={(e) => {
          e.preventDefault()
          if (!invalid && dirty) save.mutate({ enabled: cur.enabled, minutes })
        }}
      >
        <span className="flex items-center gap-2.5">
          <Switch id={`${id}-on`} checked={cur.enabled} onCheckedChange={(enabled) => edit({ enabled })} />
          <label htmlFor={`${id}-on`} className="cursor-pointer text-[13px] text-ink">
            Alert when silent
          </label>
        </span>
        <span className="flex items-center gap-2">
          <label htmlFor={`${id}-min`} className={cx('text-[13px]', cur.enabled ? 'text-ink' : 'text-faint')}>
            after
          </label>
          <input
            id={`${id}-min`}
            type="text"
            inputMode="numeric"
            value={cur.minutes}
            disabled={!cur.enabled}
            aria-label="Silent minutes"
            aria-invalid={invalid}
            aria-describedby={error ? `${id}-err` : undefined}
            onChange={(e) => edit({ minutes: e.target.value })}
            className={cx(
              'tabular h-8 w-[72px] rounded-field border bg-field px-2.5 text-right font-mono text-[13px] text-ink disabled:cursor-not-allowed disabled:opacity-50',
              invalid ? 'border-err' : 'border-field-line',
            )}
          />
          <span className={cx('text-[13px]', cur.enabled ? 'text-ink' : 'text-faint')}>minutes</span>
        </span>
        <Button type="submit" size="sm" disabled={!dirty || invalid || save.isPending}>
          {save.isPending ? 'Saving' : 'Save'}
        </Button>
        {justSaved && !dirty ? (
          <span role="status" className="text-xs text-ok">
            Saved: {saved.enabled ? `alerts after ${saved.minutes} min of silence` : 'off'}
          </span>
        ) : null}
      </form>
      {error ? (
        <p id={`${id}-err`} role="alert" className="m-0 text-xs text-err">
          {error}
        </p>
      ) : null}
    </Card>
  )
}
