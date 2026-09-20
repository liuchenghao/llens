import { ref, type Ref, watch } from 'vue'

/**
 * 数字滚动 composable（P2 体验增强，PRD §4.1）。
 *
 * 把目标数字从 0（或当前值）平滑滚动到最新值，spring 缓动，
 * 用于看板/概览的 KPI 数字，让数据刷新时有可见的过渡而非瞬间跳变。
 *
 * @param target  响应式目标值（Ref<number>）
 * @param duration 滚动时长 ms，默认 600
 * @returns 响应式显示值（已四舍五入的整数）
 */
export function useCountUp(target: Ref<number>, duration = 600): Ref<number> {
  const display = ref(0)
  let raf = 0

  // spring-like easeOutCubic：起始快、收尾缓，观感自然。
  const easeOutCubic = (t: number) => 1 - Math.pow(1 - t, 3)

  function animate(from: number, to: number) {
    cancelAnimationFrame(raf)
    if (from === to) {
      display.value = to
      return
    }
    const start = performance.now()
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / duration)
      const v = from + (to - from) * easeOutCubic(t)
      display.value = Math.round(v)
      if (t < 1) raf = requestAnimationFrame(step)
    }
    raf = requestAnimationFrame(step)
  }

  // 首次从 0 滚起，之后跟随 target 变化。
  watch(
    target,
    (to, from) => {
      animate(from ?? 0, to)
    },
    { immediate: true },
  )

  return display
}
