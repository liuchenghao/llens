<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'

type Diary = {
  date: string
  brief: string
  top3: string[]
  highlights: string[]
  todos: { text: string; done: boolean }[]
  advice: string[]
  tip: string
  status: string
  source: string[]
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
    await invoke('regenerate_diaries', { limit: 5 }).catch(() => {})
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
    const d = await invoke<Diary>('force_regenerate_day', { day: selected.value })
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
  // 自动刷新发生在用户点击查看某一天时（pickDate → loadDiarySmart）
  await loadDiaryOnly()
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

// 智能加载：若当天 10 分钟数据比已有日记多，自动重新生成；否则直接读旧日记（不耗 LLM）
async function loadDiarySmart() {
  if (!selected.value) return
  msg.value = ''
  diary.value = null
  busy.value = true
  try {
    const d = await invoke<Diary>('smart_regenerate_day', { day: selected.value })
    diary.value = d
    if (!d.source?.length) {
      msg.value = '该天暂无 10 分钟汇总数据，无法生成日记。'
    } else if (d.status === 'pending') {
      msg.value = '该天日记生成失败或尚未完成。'
    }
  } catch (e: any) {
    msg.value = String(e)
  } finally {
    busy.value = false
    dates.value = await invoke<string[]>('list_diary_dates').catch(() => [])
  }
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
        <div class="cal-dow"><span>一</span><span>二</span><span>三</span><span>四</span><span>五</span><span>六</span><span>日</span></div>
        <div class="cal">
          <div
            v-for="c in days"
            :key="c.day"
            :class="['cal-day', { out: !c.inMonth, sel: selected === c.day, dot: hasDiary.has(c.day), today: c.day === today() }]"
            @click="pickDate(c.day, c.inMonth)"
          >
            {{ c.label }}
            <span v-if="hasDiary.has(c.day)" class="dot-mark" />
          </div>
        </div>
        <div class="cal-legend">
          <span class="muted"><span class="dot-mark big" /> 已有日记</span>
          <div style="display:flex; gap:8px; align-items:center">
            <button class="btn" @click="loadDiary" :disabled="busy">刷新</button>
            <button class="btn primary" @click="forceUpdate" :disabled="busy">更新</button>
          </div>
        </div>
      </div>

      <div class="glass card diary-body">
        <div class="diary-head">
          <h3>{{ selected || '…' }} 的日记</h3>
          <span v-if="busy" class="muted">生成中…</span>
          <span v-else-if="msg" class="chip warn">{{ msg }}</span>
        </div>

        <template v-if="diary && diary.status === 'done'">
          <div class="diary-content">
            <div class="d-sec"><b>简短总结</b><p>{{ diary.brief || '—' }}</p></div>
            <div class="d-sec"><b>主要事项</b>
              <ol><li v-for="(t, i) in diary.top3" :key="i">{{ t }}</li></ol>
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
.cal-legend {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-top: 10px;
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
