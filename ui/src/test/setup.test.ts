import { getConfig } from '@testing-library/react'
import { expect, it } from 'vitest'

it('async queries wait up to 5 s, so a loaded full suite does not time them out', () => {
  expect(getConfig().asyncUtilTimeout).toBe(5_000)
})
