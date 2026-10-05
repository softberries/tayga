import { QueryClientProvider } from '@tanstack/react-query'
import { RouterProvider } from '@tanstack/react-router'
import { LazyMotion, MotionConfig, domAnimation } from 'motion/react'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { LiveProvider } from './app/live'
import { createQueryClient } from './app/queryClient'
import { TooltipProvider } from './components/ui/Tooltip'
import './index.css'
import { createAppRouter } from './router'
import { ThemeProvider } from './theme/ThemeProvider'

const queryClient = createQueryClient()
const router = createAppRouter(queryClient)

const root = document.getElementById('root')
if (!root) throw new Error('#root is missing from index.html')

createRoot(root).render(
  <StrictMode>
    <ThemeProvider>
      {/* m.* components only; `strict` throws if a full motion.* sneaks in. */}
      <LazyMotion features={domAnimation} strict>
      <MotionConfig reducedMotion="user">
        <QueryClientProvider client={queryClient}>
          <LiveProvider>
            <TooltipProvider delayDuration={300}>
              <RouterProvider router={router} />
            </TooltipProvider>
          </LiveProvider>
        </QueryClientProvider>
      </MotionConfig>
      </LazyMotion>
    </ThemeProvider>
  </StrictMode>,
)
