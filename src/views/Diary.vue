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
    // Always run the (cheap, idempotent) regenerate pass first: it only
    // re-generates days that are missing or stale (10-min data has grown
    // since the diary was written). A fresh day is a no-op.
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

onMounted(async () => {
  // pick the most recent date that has a diary, or today if none
  dates.value = await invoke<string[]>('list_diary_dates').catch(() => [])
  if (dates.value.length) {
    const sorted = [...dates.value].sort().reverse()
    selected.value = sorted[0]
  } else {
    selected.value = today()
  }
  // also shift the calendar to the month of the selected day
  const [y, m] = selected.value.split('-').map(Number)
  year.value = y
  month.value = m - 1
  await loadDiary()
})

function pickDate(day: string, inMonth: boolean) {
  if (!inMonth) {
    const [y, m] = day.split('-').map(Number)
    year.value = y
    month.value = m - 1
  }
  selected.value = day
  loadDiary()
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
  <div>
    <h1 class="page">日记</h1>
    <div class="row">
      <div class="glass card" style="flex: 1.1">
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
          <button class="btn" @click="loadDiary" :disabled="busy">刷新</button>
        </div>
      </div>

      <div class="glass card" style="flex: 1.4">
        <div class="diary-head">
          <h3>{{ selected || '…' }} 的日记</h3>
          <span v-if="busy" class="muted">生成中…</span>
          <span v-else-if="msg" class="chip warn">{{ msg }}</span>
        </div>

        <template v-if="diary && diary.status === 'done'">
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
          <div class="d-sec"><b>温馨提示</b><p class="tip">{{ diary.tip || '—' }}</p></div>
          <div class="muted" style="margin-top: 8px; font-size: 11px">
            数据源：{{ diary.source.length }} 个 10 分钟片段
          </div>
        </template>
        <div v-else-if="diary" class="muted">日记尚未生成，点击右上角「刷新」重试。</div>
        <div v-else class="muted">暂无日记</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
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
