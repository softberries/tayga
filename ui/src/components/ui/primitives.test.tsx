import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { LazyMotion, MotionConfig, domAnimation } from 'motion/react'
import { useState } from 'react'
import type { ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { ApiError } from '../../api/client'
import { Badge } from './Badge'
import { Button } from './Button'
import { Card, PanelTitle } from './Card'
import { CountUp } from './CountUp'
import { DialogContent, DialogRoot, DialogTrigger } from './Dialog'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from './DropdownMenu'
import { EmptyState } from './EmptyState'
import { ErrorState } from './ErrorState'
import { IconButton } from './IconButton'
import { Kbd } from './Kbd'
import { Popover, PopoverContent, PopoverTrigger } from './Popover'
import { ScrollArea } from './ScrollArea'
import { SHEET_MIN, Sheet } from './Sheet'
import { Skeleton } from './Skeleton'
import { StaggerItem, StaggerList } from './Stagger'
import { Tabs, TabsContent, TabsList, TabsTrigger } from './Tabs'
import { ToggleGroup } from './ToggleGroup'
import { Reveal } from './Reveal'
import { Tooltip, TooltipProvider, TruncationTooltip } from './Tooltip'

const withTooltips = (ui: ReactNode) => render(<TooltipProvider delayDuration={0}>{ui}</TooltipProvider>)

describe('Button', () => {
  it('is a type=button and fires onClick', async () => {
    const onClick = vi.fn()
    render(<Button onClick={onClick}>Open story</Button>)
    const b = screen.getByRole('button', { name: 'Open story' })
    expect(b).toHaveAttribute('type', 'button')
    await userEvent.click(b)
    expect(onClick).toHaveBeenCalledOnce()
  })
  it('renders its child with asChild', () => {
    render(
      <Button asChild variant="primary">
        <a href="/x">Go</a>
      </Button>,
    )
    const a = screen.getByRole('link', { name: 'Go' })
    expect(a).toHaveClass('tg-cta')
    expect(a).not.toHaveAttribute('type')
  })
})

describe('IconButton', () => {
  it('has an aria-label and a tooltip', async () => {
    withTooltips(<IconButton label="Refresh" icon={<svg />} />)
    const b = screen.getByRole('button', { name: 'Refresh' })
    await userEvent.hover(b)
    expect(await screen.findByRole('tooltip')).toHaveTextContent('Refresh')
  })
})

describe('Badge', () => {
  it.each([
    ['error', 'text-err'],
    ['slow', 'text-slow'],
    ['new', 'text-on-accent'],
    ['spike', 'text-on-slow'],
    ['ok', 'text-ok'],
  ] as const)('%s uses its tokens', (kind, cls) => {
    render(<Badge kind={kind}>{kind}</Badge>)
    expect(screen.getByText(kind)).toHaveClass(cls)
  })
  it('pill with a pulsing dot', () => {
    const { container } = render(
      <Badge kind="error" shape="pill" pulse>
        2 services degraded
      </Badge>,
    )
    expect(screen.getByText('2 services degraded')).toHaveClass('rounded-full')
    expect(container.querySelector('.tg-pulse')).not.toBeNull()
  })
})

describe('Kbd', () => {
  it('renders a kbd element', () => {
    render(<Kbd>⌘K</Kbd>)
    expect(screen.getByText('⌘K').tagName).toBe('KBD')
  })
})

describe('Card', () => {
  it('panel, tile and inner variants', () => {
    render(
      <>
        <Card data-testid="p">
          <PanelTitle>Service map</PanelTitle>
        </Card>
        <Card data-testid="t" variant="tile" elevated lift />
        <Card data-testid="i" variant="inner" />
      </>,
    )
    expect(screen.getByTestId('p')).toHaveClass('rounded-panel', 'shadow-panel')
    expect(screen.getByTestId('t')).toHaveClass('rounded-card', 'shadow-panel-lg')
    expect(screen.getByTestId('i')).not.toHaveClass('shadow-panel')
    expect(screen.getByRole('heading', { name: 'Service map' })).toBeInTheDocument()
  })
})

describe('Skeleton', () => {
  it('is decorative', () => {
    const { container } = render(<Skeleton className="h-4" />)
    expect(container.firstChild).toHaveAttribute('aria-hidden', 'true')
    expect(container.firstChild).toHaveClass('tg-skeleton')
  })
})

describe('Tooltip', () => {
  it('opens on focus', async () => {
    withTooltips(
      <Tooltip content="Stories">
        <button type="button">s</button>
      </Tooltip>,
    )
    act(() => screen.getByRole('button').focus())
    expect(await screen.findByRole('tooltip')).toHaveTextContent('Stories')
  })
})

describe('TruncationTooltip', () => {
  /** jsdom has no layout: report a cut-off (or not) through the size properties. */
  function layout(scrollWidth: number, clientWidth: number) {
    vi.spyOn(HTMLElement.prototype, 'scrollWidth', 'get').mockReturnValue(scrollWidth)
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(clientWidth)
  }
  const cell = () =>
    withTooltips(
      <TruncationTooltip content="a very long endpoint name">
        <button type="button" className="truncate">
          a very long end…
        </button>
      </TruncationTooltip>,
    )

  it('shows the full text when the text is cut off', async () => {
    layout(300, 120)
    cell()
    act(() => screen.getByRole('button').focus())
    expect(await screen.findByRole('tooltip')).toHaveTextContent('a very long endpoint name')
  })
  it('stays closed when the text fits', async () => {
    layout(100, 120)
    cell()
    act(() => screen.getByRole('button').focus())
    await new Promise((r) => setTimeout(r, 30))
    expect(screen.queryByRole('tooltip')).toBeNull()
  })
})

describe('TruncationTooltip keyboard', () => {
  it('opens when the focusable row it sits in gets focus, only if truncated', async () => {
    vi.spyOn(HTMLElement.prototype, 'scrollWidth', 'get').mockReturnValue(300)
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(100)
    withTooltips(
      <div role="row" tabIndex={0}>
        <TruncationTooltip content="the whole summary" openOnHostFocus>
          <span className="truncate">the whole…</span>
        </TruncationTooltip>
      </div>,
    )
    act(() => screen.getByRole('row').focus())
    expect(await screen.findByRole('tooltip')).toHaveTextContent('the whole summary')
    act(() => screen.getByRole('row').blur())
    await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull())
  })
})

describe('Reveal', () => {
  it('fades in from transparent, and renders at full opacity under reduced motion', () => {
    const motion = (mode: 'never' | 'always', id: string) => (
      <LazyMotion features={domAnimation} strict>
        <MotionConfig reducedMotion={mode}>
          <Reveal data-testid={id}>x</Reveal>
        </MotionConfig>
      </LazyMotion>
    )
    render(motion('never', 'r'))
    expect(screen.getByTestId('r').style.opacity).toBe('0')
    render(motion('always', 'r2'))
    expect(screen.getByTestId('r2').style.opacity).toBe('1')
  })
})

describe('Dialog', () => {
  it('opens, has a title and closes on Escape', async () => {
    const user = userEvent.setup()
    render(
      <DialogRoot>
        <DialogTrigger>open</DialogTrigger>
        <DialogContent title="Command palette">body</DialogContent>
      </DialogRoot>,
    )
    await user.click(screen.getByText('open'))
    expect(screen.getByRole('dialog', { name: 'Command palette' })).toBeInTheDocument()
    await user.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
  })
})

function SheetHarness() {
  const [open, setOpen] = useState(true)
  return (
    <MotionConfig reducedMotion="always">
      <Sheet open={open} onOpenChange={setOpen} title="Span" defaultWidth={500} storageKey="w">
        details
      </Sheet>
    </MotionConfig>
  )
}

describe('Sheet', () => {
  it('slides in, resizes by keyboard, remembers the width, closes', async () => {
    const user = userEvent.setup()
    render(<SheetHarness />)
    const sheet = screen.getByRole('dialog', { name: 'Span' })
    expect(sheet).toHaveStyle({ width: '500px' })
    const handle = screen.getByRole('separator', { name: 'Resize panel' })
    fireEvent.keyDown(handle, { key: 'ArrowLeft' })
    expect(sheet).toHaveStyle({ width: '532px' })
    expect(handle).toHaveAttribute('aria-valuenow', '532')
    expect(window.localStorage.getItem('w')).toBe('532')
    fireEvent.keyDown(handle, { key: 'End' })
    expect(sheet).toHaveStyle({ width: `${SHEET_MIN}px` })
    await user.click(screen.getByRole('button', { name: 'Close panel' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
  })
  it('resizes by dragging the left edge', () => {
    render(<SheetHarness />)
    const handle = screen.getByRole('separator')
    fireEvent.pointerDown(handle, { clientX: 600, pointerId: 1 })
    fireEvent.pointerMove(handle, { clientX: 500, pointerId: 1 })
    fireEvent.pointerUp(handle, { clientX: 500, pointerId: 1 })
    expect(screen.getByRole('dialog')).toHaveStyle({ width: '600px' })
  })
})

describe('Tabs', () => {
  it('switches panels', async () => {
    render(
      <Tabs defaultValue="a">
        <TabsList aria-label="Logs">
          <TabsTrigger value="a">Alerts</TabsTrigger>
          <TabsTrigger value="b">Templates</TabsTrigger>
        </TabsList>
        <TabsContent value="a">alerts panel</TabsContent>
        <TabsContent value="b">templates panel</TabsContent>
      </Tabs>,
    )
    expect(screen.getByRole('tabpanel')).toHaveTextContent('alerts panel')
    await userEvent.click(screen.getByRole('tab', { name: 'Templates' }))
    expect(screen.getByRole('tabpanel')).toHaveTextContent('templates panel')
  })
})

describe('ToggleGroup', () => {
  it('selects one value and never deselects', async () => {
    const onChange = vi.fn()
    render(
      <ToggleGroup
        label="Time range"
        options={[
          { value: '15m', label: '15m' },
          { value: '1h', label: '1h' },
        ]}
        value="1h"
        onValueChange={onChange}
      />,
    )
    expect(screen.getByRole('radiogroup', { name: 'Time range' })).toBeInTheDocument()
    expect(screen.getByRole('radio', { name: '1h' })).toHaveAttribute('data-state', 'on')
    await userEvent.click(screen.getByRole('radio', { name: '15m' }))
    expect(onChange).toHaveBeenCalledWith('15m')
    onChange.mockClear()
    await userEvent.click(screen.getByRole('radio', { name: '1h' }))
    expect(onChange).not.toHaveBeenCalled()
  })
})

describe('Popover', () => {
  it('opens its content', async () => {
    render(
      <Popover>
        <PopoverTrigger>filter</PopoverTrigger>
        <PopoverContent>service list</PopoverContent>
      </Popover>,
    )
    await userEvent.click(screen.getByText('filter'))
    expect(screen.getByText('service list')).toBeInTheDocument()
  })
})

describe('DropdownMenu', () => {
  it('opens with the keyboard and selects an item', async () => {
    const user = userEvent.setup()
    const onSelect = vi.fn()
    render(
      <DropdownMenu>
        <DropdownMenuTrigger>kind</DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuItem onSelect={onSelect}>Errors</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>,
    )
    act(() => screen.getByText('kind').focus())
    await user.keyboard('{Enter}')
    await user.click(await screen.findByRole('menuitem', { name: 'Errors' }))
    expect(onSelect).toHaveBeenCalledOnce()
  })
})

describe('ScrollArea', () => {
  it('renders a focusable labelled viewport', () => {
    render(<ScrollArea label="Logs">rows</ScrollArea>)
    const vp = screen.getByLabelText('Logs')
    expect(vp).toHaveAttribute('tabindex', '0')
    expect(vp).toHaveTextContent('rows')
  })
})

describe('EmptyState', () => {
  it('shows title, description and action', () => {
    render(<EmptyState title="No stories" description="Nothing failed in the last hour." action={<button>Reset</button>} />)
    expect(screen.getByText('No stories')).toBeInTheDocument()
    expect(screen.getByText('Nothing failed in the last hour.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Reset' })).toBeInTheDocument()
  })
})

describe('ErrorState', () => {
  it('explains a 503 and retries', async () => {
    const onRetry = vi.fn()
    render(<ErrorState error={new ApiError(503, 'clickhouse unavailable')} onRetry={onRetry} />)
    const alert = screen.getByRole('alert')
    expect(alert).toHaveTextContent('Storage is unavailable')
    expect(alert).toHaveTextContent('503 · clickhouse unavailable')
    await userEvent.click(screen.getByRole('button', { name: 'Try again' }))
    expect(onRetry).toHaveBeenCalledOnce()
  })
  it('handles plain errors', () => {
    render(<ErrorState error={new Error('boom')} />)
    expect(screen.getByRole('alert')).toHaveTextContent('Something went wrong')
  })
})

describe('CountUp', () => {
  it('counts up to the value', async () => {
    const { container } = render(<CountUp value={96} duration={0.05} />)
    expect(screen.getByText('96', { selector: '.sr-only' })).toBeInTheDocument()
    await waitFor(() => expect(container.querySelector('[aria-hidden]')).toHaveTextContent('96'))
  })
  it('shows the value at once under reduced motion', () => {
    const { container } = render(
      <MotionConfig reducedMotion="always">
        <CountUp value={1900} format={(n) => `${(n / 1000).toFixed(1)}k`} />
      </MotionConfig>,
    )
    expect(container.querySelector('[aria-hidden]')).toHaveTextContent('1.9k')
  })
})

describe('Stagger', () => {
  const list = (mode: 'never' | 'always') => (
    <LazyMotion features={domAnimation} strict>
      <MotionConfig reducedMotion={mode}>
        <StaggerList role="list">
          <StaggerItem role="listitem">a</StaggerItem>
        </StaggerList>
      </MotionConfig>
    </LazyMotion>
  )
  it('fades items in from transparent, but starts them visible under reduced motion', () => {
    const first = render(list('never'))
    expect(screen.getByRole('listitem').style.opacity).toBe('0')
    first.unmount()
    render(list('always'))
    expect(screen.getByRole('listitem').style.opacity).toBe('1')
  })
  it('renders every item', () => {
    render(
      <StaggerList role="list">
        {['a', 'b', 'c'].map((x) => (
          <StaggerItem role="listitem" key={x}>
            {x}
          </StaggerItem>
        ))}
      </StaggerList>,
    )
    expect(screen.getAllByRole('listitem')).toHaveLength(3)
  })
})
