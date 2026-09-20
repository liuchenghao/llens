<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { invoke } from '@tauri-apps/api/core'

type Task = {
  text: string
  minutes: number
}

type Diary = {
  date: string
  brief: string
  top3: Task[]
  highlights: string[]
  todos: { text: string; done: boolean }[]
  advice: string[]
  tip: string
  status: string
  source: string[]
}

// 主要事项统计阈值（分钟）：只显示累计时长 ≥ 该值的任务
const minMinutes = ref<number>(30)
const filteredTop3 = computed(() =>
  (diary.value?.top3 || []).filter((t) => t.minutes >= minMinutes.value)
)

// 待生成日期：有 10min 数据但无日记（或已过期），日历用琥珀色标记 + 自动补生成
const pendingDays = ref<Set<string>>(new Set())
const catchupRunning = ref(false)
const catchupDone = ref(false)

// 刷新 days_with_slices → 计算 pendingDays（有数据但 has_diary=false 的天）
async function refreshPending() {
  const days = await invoke<{ date: string; slices: number; has_diary: boolean }[]>(
    'days_with_slices',
  ).catch(() => [])
  pendingDays.value = new Set(days.filter((d) => !d.has_diary).map((d) => d.date))
}

// 自动补生成：把当前 pendingDays 里的日期（从旧到新）逐个生成，显示 spinner
async function autoCatchup() {
  if (catchupRunning.value || pendingDays.value.size === 0) return
  catchupRunning.value = true
  catchupDone.value = false
  const days: string[] = [...pendingDays.value].sort()
  for (const day of days) {
    try {
      await invoke('force_regenerate_day', { day, minTaskMinutes: minMinutes.value })
    } catch {
      /* 单天失败不阻断其余 */
    }
  }
  catchupRunning.value = false
  catchupDone.value = true
  await refreshPending()
  if (selected.value && pendingDays.value.has(selected.value)) {
    await loadDiaryOnly()
  }
}

const today = () => {
  const d = new Date()
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

const selected = ref<string | null>(null)   // resolved after loadDiaryDates
const diary = ref<Diary | null>(null)
const busy = ref(false)
const msg = ref('')
const dates = ref<string[]>([])

const year = ref(new Date().getFullYear())
const month = ref(new Date().getMonth()) // 0-based

// calendar grid: weeks x days
const days = computed(() => {
  const y = year.value
  const m = month.value
  const first = new Date(y, m, 1)
  const startWeekday = (first.getDay() + 6) % 7 // Mon=0
  const daysInMonth = new Date(y, m + 1, 0).getDate()
  const cells: { label: string; day: string; inMonth: boolean }[] = []
  for (let i = 0; i < startWeekday; i++) {
    const d = new Date(y, m, -startWeekday + i + 1)
    cells.push({
      label: String(d.getDate()),
      day: fmt(d),
      inMonth: false,
    })
  }
  for (let d = 1; d <= daysInMonth; d++) {
    cells.push({ label: String(d), day: fmt(new Date(y, m, d)), inMonth: true })
  }
  while (cells.length % 7 !== 0) {
    const d = new Date(y, m + 1, cells.length - startWeekday - daysInMonth + 1)
    cells.push({ label: String(d.getDate()), day: fmt(d), inMonth: false })
  }
  return cells
})

function fmt(d: Date) {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

const hasDiary = computed(() => new Set(dates.value))

async function loadDiary() {
  if (!selected.value) return
  msg.value = ''
  diary.value = null
  busy.value = true
  try {
    // 按选中日期做增量更新：只补生成过期或缺失的日记，已最新的是 no-op
    await invoke('regenerate_diaries', {
      limit: 5,
      minTaskMinutes: minMinutes.value,
    }).catch(() => {})
    const d = await invoke<Diary | null>('get_diary', { day: selected.value })
    diary.value = d
    if (!d) {
      msg.value = '该天暂无 10 分钟汇总数据，无法生成日记。'
    }
  } catch (e: any) {
    msg.value = String(e)
  } finally {
    busy.value = false
    dates.value = await invoke<string[]>('list_diary_dates').catch(() => [])
  }
}

// 强制刷新：按选中日期无条件重跑 LLM，覆盖已有日记并保存（「更新」按钮手动触发）
async function forceUpdate() {
  if (!selected.value) return
  msg.value = ''
  busy.value = true
  try {
    const d = await invoke<Diary>('force_regenerate_day', {
      day: selected.value,
      minTaskMinutes: minMinutes.value,
    })
    diary.value = d
    if (!d || !d.source?.length) {
      msg.value = '该天暂无 10 分钟数据，无法生成日记。'
    } else {
      msg.value = '已强制更新并保存。'
    }
  } catch (e: any) {
    msg.value = String(e)
  } finally {
    busy.value = false
    dates.value = await invoke<string[]>('list_diary_dates').catch(() => [])
  }
}

onMounted(async () => {
  dates.value = await invoke<string[]>('list_diary_dates').catch(() => [])
  // 默认进入「今天」；而非磁盘上最新一篇日记的日期
  // （今天通常还没有生成日记，此时右侧显示“暂无日记”并可点刷新生成）
  selected.value = today()
  // 日历切到今天所在月份
  const [y, m] = selected.value.split('-').map(Number)
  year.value = y
  month.value = m - 1
  // 进入页面：只读磁盘已有日记，不触发 LLM（符合「进入不自动刷新」）
  await loadDiaryOnly()
  // PRD §4.3：若存在「有 10min 数据但无日记」的天，后台逐个补生成 + 日历 spinner
  refreshPending().then(() => autoCatchup())
})

// 问答跳转：监听 window 的 llens_jump 事件，定位到指定日期
let jumpHandler: ((e: Event) => void) | null = null
onMounted(() => {
  jumpHandler = (e: Event) => {
    const detail = (e as CustomEvent).detail
    if (detail?.time && detail.time !== selected.value) pickDate(detail.time, true)
  }
  window.addEventListener('llens_jump', jumpHandler)
})
onBeforeUnmount(() => {
  if (jumpHandler) window.removeEventListener('llens_jump', jumpHandler)
})

function pickDate(day: string, inMonth: boolean) {
  if (!inMonth) {
    const [y, m] = day.split('-').map(Number)
    year.value = y
    month.value = m - 1
  }
  selected.value = day
  // 点击日期只切换查看，不触发 LLM 生成；读磁盘已有日记
  loadDiaryOnly()
}

async function loadDiaryOnly() {
  if (!selected.value) return
  msg.value = ''
  diary.value = null
  try {
    const d = await invoke<Diary | null>('get_diary', { day: selected.value })
    diary.value = d
    if (!d) {
      msg.value = '该天暂无日记。点「刷新」生成或「更新」强制重写。'
    }
  } catch (e: any) {
    msg.value = String(e)
  } finally {
    dates.value = await invoke<string[]>('list_diary_dates').catch(() => [])
    refreshPending()
  }
}

function shiftMonth(delta: number) {
  let m = month.value + delta
  let y = year.value
  if (m < 0) { m = 11; y-- }
  if (m > 11) { m = 0; y++ }
  month.value = m
  year.value = y
}
</script>

<template>
  <div class="diary-root">
    <h1 class="page">日记</h1>
    <div class="row">
      <div class="glass card diary-cal">
        <div class="cal-head">
          <button class="btn" @click="shiftMonth(-1)">‹</button>
          <div class="cal-title">{{ year }} 年 {{ month + 1 }} 月</div>
          <button class="btn" @click="shiftMonth(1)">›</button>
        </div>
        <div class="cal-body">
          <div class="cal-dow"><span>一</span><span>二</span><span>三</span><span>四</span><span>五</span><span>六</span><span>日</span></div>
          <div class="cal">
            <div
              v-for="c in days"
              :key="c.day"
              :class="['cal-day', { out: !c.inMonth, sel: selected === c.day, dot: hasDiary.has(c.day), pending: pendingDays.has(c.day), today: c.day === today() }]"
              @click="pickDate(c.day, c.inMonth)"
            >
              {{ c.label }}
              <span v-if="hasDiary.has(c.day)" class="dot-mark" />
              <span v-else-if="pendingDays.has(c.day)" class="dot-mark pending-mark" />
            </div>
          </div>
        </div>
        <div class="cal-legend">
          <span class="muted"><span class="dot-mark big" /> 已有日记</span>
          <span class="muted" style="display:inline-flex;align-items:center;gap:4px"><span class="dot-mark big pending-mark" /> 待生成</span>
          <div style="display:flex; gap:8px; align-items:center">
            <button class="btn" @click="loadDiary" :disabled="busy || catchupRunning">刷新</button>
            <button class="btn primary" @click="forceUpdate" :disabled="busy || catchupRunning">更新</button>
            <button class="btn" @click="autoCatchup" :disabled="catchupRunning || pendingDays.size === 0">
              {{ catchupRunning ? '补生成中…' : (pendingDays.size ? `补生成 ${pendingDays.size}` : '补生成') }}
            </button>
            <label class="min-minutes" title="控制 LLM 生成主要事项时统计的最低任务时长">
              仅 ≥
              <input
                v-model.number="minMinutes"
                type="number"
                min="0"
                step="5"
                class="min-minutes-input"
              />
              分钟
            </label>
          </div>
        </div>
      </div>

      <div class="glass card diary-body">
        <div class="diary-head">
          <h3>{{ selected || '…' }} 的日记</h3>
          <span v-if="busy" class="muted">生成中…</span>
          <span v-else-if="catchupRunning" class="chip" style="background:rgba(251,191,36,0.15);border:1px solid rgba(251,191,36,0.4);color:#fbbf24">补生成中…</span>
          <span v-else-if="catchupDone && !pendingDays.size" class="chip ok">已补齐待生成日记</span>
          <span v-else-if="msg" class="chip warn">{{ msg }}</span>
        </div>

        <template v-if="diary && diary.status === 'done'">
          <div class="diary-content">
            <div class="d-sec"><b>简短总结</b><p>{{ diary.brief || '—' }}</p></div>
            <div class="d-sec">
              <b>主要事项</b>
              <ol class="top3-list">
                <li v-for="(t, i) in filteredTop3" :key="i">
                  {{ t.text }}
                  <span v-if="t.minutes > 0" class="task-dur">约 {{ t.minutes }} 分钟</span>
                </li>
                <li v-if="!filteredTop3.length" class="muted top3-empty">无符合条件的任务</li>
              </ol>
            </div>
            <div class="d-sec"><b>高光</b>
              <ul><li v-for="(h, i) in diary.highlights" :key="i">✦ {{ h }}</li></ul>
            </div>
            <div class="d-sec"><b>待办</b>
              <ul class="todos">
                <li v-for="(t, i) in diary.todos" :key="i">
                  <input type="checkbox" :checked="t.done" /> {{ t.text }}
                </li>
              </ul>
            </div>
            <div class="d-sec"><b>优化建议</b>
              <ul><li v-for="(a, i) in diary.advice" :key="i">{{ a }}</li></ul>
            </div>
          </div>
          <!-- 温馨提示 + 数据源：固定在卡片最底部，不随内容区滚动 -->
          <div class="diary-foot">
            <div class="d-sec foot-sec"><b>温馨提示</b><p class="tip">{{ diary.tip || '—' }}</p></div>
            <div class="muted foot-src">数据源：{{ diary.source.length }} 个 10 分钟片段</div>
          </div>
        </template>
        <div v-else-if="diary" class="muted">日记尚未生成，点击「刷新」或「更新」重试。</div>
        <div v-else class="muted">暂无日记</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* ── 高度自适应：根容器填满 .content 剩余高度，行内卡片随窗口伸缩 ── */
.diary-root {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
}
.diary-root .row {
  flex: 1;
  min-height: 0;
  align-items: stretch;
  overflow: hidden;
  flex-wrap: nowrap;   /* 覆盖全局 .row 的 flex-wrap: wrap，保证两卡横向并排 */
}
/* 日历卡片：内容区可内部滚动（窗口很矮时） */
.diary-cal {
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
  flex: 0 0 42%;   /* 固定占 42%，不随内容伸缩，保证与右侧并排 */
  min-width: 0;
}
.diary-cal .cal-body {
  flex: 1;
  overflow: hidden;
}
.diary-cal .cal {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
}
.diary-cal .cal::-webkit-scrollbar { width: 4px; }
.diary-cal .cal::-webkit-scrollbar-thumb {
  background: rgba(255,255,255,0.14);
  border-radius: 4px;
}
/* 日记正文卡片：填满窗口剩余高度，内容区内部滚动 */
.diary-body {
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
  flex: 1 1 0%;   /* 占剩余全部宽度 */
  min-width: 0;
}
/* 标题栏固定不滚动 */
.diary-head {
  flex-shrink: 0;
}
/* 简短总结以下的内容区：占满剩余高度，超出后内部滚动 */
.diary-content {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  padding-right: 6px;
}
.diary-content::-webkit-scrollbar { width: 6px; }
.diary-content::-webkit-scrollbar-thumb {
  background: rgba(255,255,255,0.16);
  border-radius: 4px;
}
/* 温馨提示 + 数据源：固定在卡片最底部，不随内容区滚动 */
.diary-foot {
  flex-shrink: 0;
  border-top: 1px solid rgba(255,255,255,0.08);
  padding-top: 10px;
  margin-top: 8px;
}
.diary-foot .foot-sec {
  margin: 0;
}
.diary-foot .foot-src {
  font-size: 11px;
  margin-top: 6px;
}

/* 窗口很矮时，允许整页 .content 兜底滚动（.row overflow 恢复可见） */
@media (max-height: 520px) {
  .diary-root .row {
    overflow: visible;
    flex: none;
  }
}

.cal-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
}
.cal-title {
  font-weight: 700;
  font-size: 15px;
}
.cal {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  gap: 4px;
}
.cal-dow {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  gap: 4px;
  font-size: 11px;
  color: #8b90b5;
  margin-bottom: 4px;
}
.cal-dow span {
  text-align: center;
}
.cal-day {
  position: relative;
  text-align: center;
  padding: 7px 0;
  border-radius: 10px;
  font-size: 13px;
  cursor: pointer;
  border: 1px solid transparent;
  transition: all 0.15s;
}
.cal-day:hover {
  background: rgba(255, 255, 255, 0.08);
}
.cal-day.out {
  color: #5a5f8a;
}
.cal-day.sel {
  background: rgba(90, 140, 255, 0.25);
  border-color: rgba(90, 140, 255, 0.6);
}
.cal-day.today {
  font-weight: 800;
  color: #8fd6ff;
}
.dot-mark {
  position: absolute;
  bottom: 3px;
  left: 50%;
  transform: translateX(-50%);
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: #6ee7b7;
}
.dot-mark.big {
  position: static;
  display: inline-block;
  transform: none;
  width: 8px;
  height: 8px;
  vertical-align: middle;
  margin-right: 4px;
}
/* 待生成标记：琥珀色，区分于已完成的绿色 */
.dot-mark.pending-mark {
  background: #fbbf24;
}
.cal-legend {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-top: 10px;
}
/* 日历卡片内按钮：紧凑尺寸，与分钟输入框高度对齐 */
.cal-legend .btn {
  padding: 5px 12px;
  font-size: 12px;
  border-radius: 8px;
}
.diary-head {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 8px;
  flex-shrink: 0;
}
.diary-head h3 {
  margin: 0;
}
.d-sec {
  margin: 12px 0;
}
.d-sec b {
  display: block;
  font-size: 12px;
  color: #aeb4dc;
  letter-spacing: 0.06em;
  margin-bottom: 6px;
}
.d-sec p,
.d-sec li {
  font-size: 14px;
  line-height: 1.6;
  margin: 4px 0;
}
.d-sec ul,
.d-sec ol {
  margin: 0;
  padding-left: 20px;
}
/* 日报条目序号：有多少写多少（连续数字编号） */
.top3-list {
  counter-reset: top3;
  padding-left: 20px;
}
.top3-list li {
  counter-increment: top3;
  list-style: none;
  position: relative;
  padding-left: 24px;
  margin: 6px 0;
}
.top3-list li::before {
  content: counter(top3) ".";
  position: absolute;
  left: 0;
  color: #aeb4dc;
  font-weight: 600;
}
/* 任务累计时长标识 */
.task-dur {
  display: inline-block;
  margin-left: 8px;
  font-size: 11px;
  color: #8fd6ff;
  background: rgba(143,214,255,0.12);
  border: 1px solid rgba(143,214,255,0.25);
  border-radius: 8px;
  padding: 1px 6px;
  vertical-align: middle;
}
/* 主要事项标题行：标题 + 分钟阈值输入框同行右对齐 */
.top3-head {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 2px;
}
.top3-head b {
  display: block;
  font-size: 12px;
  color: #aeb4dc;
  letter-spacing: 0.06em;
}
.min-minutes {
  margin-left: auto;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  color: #8b90b5;
}
.min-minutes-input {
  width: 64px;
  padding: 2px 6px;
  background: rgba(255,255,255,0.07);
  border: 1px solid rgba(255,255,255,0.16);
  border-radius: 6px;
  color: #fff;
  font-size: 12px;
  outline: none;
}
.min-minutes-input:focus {
  border-color: rgba(122,165,255,0.7);
  background: rgba(122,165,255,0.1);
}
.top3-empty {
  font-size: 12px;
  padding-left: 4px;
}
.tip {
  color: #9be8c8;
}
.todos li {
  display: flex;
  gap: 8px;
  align-items: center;
  list-style: none;
  padding: 4px 0;
}
.todos li input[type='checkbox'] {
  width: 16px;
  height: 16px;
  accent-color: #7aa5ff;
  cursor: pointer;
  flex-shrink: 0;
}
.todos li span {
  flex: 1;
}
.todos {
  padding-left: 4px !important;
}
</style>
