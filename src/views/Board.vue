<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, computed, nextTick } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import * as echarts from 'echarts'

type Frame = {
  time: string
  image: string
  summary5: string[]
  activity?: string
  app?: string
  project?: string
  rest?: boolean
}
type T10 = {
  bucket: number
  from: string
  to: string
  narrative: string
  activities: string[]
  top_apps: string[]
  category_breakdown: Record<string, number>
  frame_count: number
}

type Gran = 'day' | 'week' | 'month' | 'year'
const gran = ref<Gran>('day')

const range = computed((): { startDay: string; endDay: string } => {
  const now = new Date()
  const pad = (n: number) => String(n).padStart(2, '0')
  const f = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
  let start: Date
  switch (gran.value) {
    case 'day':
      start = new Date(now.getFullYear(), now.getMonth(), now.getDate())
      break
    case 'week': {
      const d = new Date()
      const dow = (d.getDay() + 6) % 7
      start = new Date(d.getFullYear(), d.getMonth(), d.getDate() - dow)
      break
    }
    case 'month':
      start = new Date(now.getFullYear(), now.getMonth(), 1)
      break
    default:
      start = new Date(now.getFullYear(), 0, 1)
  }
  return {
    startDay: f(start),
    endDay: f(new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1)),
  }
})

const frames = ref<Frame[]>([])
const t10 = ref<T10[]>([])
// 「逐日帧量趋势」独立数据源：固定最近 14 天，不受顶部粒度切换影响
const trendFrames = ref<Frame[]>([])
const errMsg = ref('')

async function load() {
  errMsg.value = ''
  try {
    const data = await invoke<{ frames: Frame[]; t10: T10[] }>('list_range', range.value)
    frames.value = data.frames
    t10.value = data.t10
    // 趋势图固定拉取最近 14 天的帧数据（只取帧，用于按天分组）
    const now = new Date()
    const pad = (n: number) => String(n).padStart(2, '0')
    const fdate = (d: Date) =>
      `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
    const start = new Date(now)
    start.setDate(start.getDate() - 13)
    const trendData = await invoke<{ frames: Frame[] }>('list_range', {
      startDay: fdate(start),
      endDay: fdate(new Date(now.getTime() + 86400000)),
    })
    trendFrames.value = trendData.frames
    await nextTick()
    renderAll()
  } catch (e: any) {
    errMsg.value = String(e)
  }
}

onMounted(load)

// ---- aggregations ----
const activityPie = computed(() => {
  const m: Record<string, number> = {}
  for (const f of frames.value) m[f.activity || '未分类'] = (m[f.activity || '未分类'] || 0) + 1
  return Object.entries(m).map(([name, value]) => ({ name, value }))
})

const appRank = computed((): [string, number][] => {
  const m: Record<string, number> = {}
  for (const f of frames.value) {
    const k = f.app && f.app !== 'unknown' ? f.app : '未识别'
    m[k] = (m[k] || 0) + 1
  }
  return Object.entries(m)
    .sort((a, b) => b[1] - a[1])
    .slice(0, 10)
})

const projectBars = computed(() => {
  const m: Record<string, number> = {}
  for (const f of frames.value) {
    const k = f.project && f.project !== 'unknown' ? f.project : '未识别'
    m[k] = (m[k] || 0) + 1
  }
  return Object.entries(m)
    .map(([name, value]) => ({ name, value: Math.round((value * 20) / 60) })) // frames*20s in minutes
    .sort((a, b) => b.value - a.value)
    .slice(0, 8)
})

const heatmap = computed(() => {
  // hours (0-23) x weekday (Mon-Sun)
  const cells: number[][] = Array.from({ length: 7 }, () => Array(24).fill(0))
  for (const f of frames.value) {
    const d = new Date(f.time)
    if (isNaN(d.getTime())) continue
    const wd = (d.getDay() + 6) % 7
    const h = d.getHours()
    cells[wd][h] += 1
  }
  return cells
})

const dailyTrend = computed(() => {
  const byDay: Record<string, number> = {}
  for (const f of trendFrames.value) {
    const d = f.time.slice(0, 10)
    byDay[d] = (byDay[d] || 0) + 1
  }
  return Object.entries(byDay).sort((a, b) => a[0].localeCompare(b[0])).map(([name, value]) => ({ name, value }))
})

// ---- charts ----
const elPie = ref<HTMLElement>()
const elApp = ref<HTMLElement>()
const elProj = ref<HTMLElement>()
const elHeat = ref<HTMLElement>()
const elTrend = ref<HTMLElement>()
let charts: echarts.ECharts[] = []

function baseOpt() {
  return {
    backgroundColor: 'transparent',
    textStyle: { color: '#c9cde8', fontSize: 12 },
    grid: { left: 40, right: 20, top: 30, bottom: 30, containLabel: true },
  }
}

function renderAll() {
  charts.forEach((c) => c.dispose())
  charts = []
  const glass = {
    textStyle: { color: '#c9cde8', fontSize: 12 },
  }
  if (elPie.value) {
    const c = echarts.init(elPie.value)
    c.setOption({
      ...glass,
      tooltip: { trigger: 'item' },
      legend: { bottom: 0, textStyle: { color: '#c9cde8' } },
      series: [
        {
          type: 'pie',
          radius: ['42%', '72%'],
          center: ['50%', '45%'],
          itemStyle: { borderRadius: 8, borderColor: 'rgba(4,6,20,.6)', borderWidth: 2 },
          label: { color: '#d5d9f5' },
          data: activityPie.value.length ? activityPie.value : [{ name: '暂无数据', value: 1 }],
        },
      ],
    })
    charts.push(c)
  }
  if (elApp.value) {
    const c = echarts.init(elApp.value)
    const data: [string, number][] = appRank.value.length
      ? appRank.value
      : [ ['暂无数据', 0] as [string, number] ]
    c.setOption({
      ...baseOpt(),
      ...glass,
      tooltip: { trigger: 'axis' },
      xAxis: { type: 'category', data: data.map((d) => d[0]), axisLabel: { color: '#c9cde8' } },
      yAxis: { type: 'value', axisLabel: { color: '#c9cde8' } },
      series: [
        {
          type: 'bar',
          data: data.map((d) => d[1]),
          itemStyle: {
            borderRadius: [0, 6, 6, 0],
            color: new echarts.graphic.LinearGradient(0, 0, 1, 0, [
              { offset: 0, color: '#5a8cff' },
              { offset: 1, color: '#c084fc' },
            ]),
          },
          barMaxWidth: 28,
        },
      ],
    })
    charts.push(c)
  }
  if (elProj.value) {
    const c = echarts.init(elProj.value)
    const data = projectBars.value.length ? projectBars.value : [{ name: '暂无数据', value: 0 }]
    c.setOption({
      ...baseOpt(),
      ...glass,
      tooltip: { trigger: 'axis' },
      xAxis: { type: 'category', data: data.map((d) => d.name), axisLabel: { color: '#c9cde8' } },
      yAxis: { type: 'value', name: '分钟', axisLabel: { color: '#c9cde8' } },
      series: [
        {
          type: 'bar',
          data: data.map((d) => d.value),
          itemStyle: { borderRadius: [6, 6, 0, 0], color: 'rgba(90,200,255,.8)' },
          barMaxWidth: 26,
        },
      ],
    })
    charts.push(c)
  }
  if (elHeat.value) {
    const c = echarts.init(elHeat.value)
    const data: number[][] = []
    heatmap.value.forEach((row, wd) =>
      row.forEach((v, h) => data.push([h, wd, v])),
    )
    c.setOption({
      ...glass,
      tooltip: {},
      grid: { left: 40, right: 20, top: 20, bottom: 40, containLabel: false },
      xAxis: {
        type: 'category',
        data: Array.from({ length: 24 }, (_, i) => `${i}`),
        axisLabel: { color: '#8b90b5', interval: 2 },
      },
      yAxis: {
        type: 'category',
        data: ['周一', '周二', '周三', '周四', '周五', '周六', '周日'],
        axisLabel: { color: '#c9cde8' },
      },
      visualMap: {
        min: 0,
        max: Math.max(1, ...heatmap.value.flat()),
        calculable: true,
        orient: 'horizontal',
        left: 'center',
        bottom: 0,
        inRange: { color: ['rgba(255,255,255,.04)', '#5a8cff', '#c084fc', '#ff8f9a'] },
        textStyle: { color: '#c9cde8' },
      },
      series: [{ type: 'heatmap', data, label: { show: false } }],
    })
    charts.push(c)
  }
  if (elTrend.value) {
    const c = echarts.init(elTrend.value)
    const data = dailyTrend.value.length ? dailyTrend.value : [{ name: '暂无数据', value: 0 }]
    c.setOption({
      ...baseOpt(),
      ...glass,
      tooltip: { trigger: 'axis' },
      xAxis: {
        type: 'category',
        data: data.map((d) => d.name),
        axisLabel: { color: '#8b90b5', rotate: data.length > 10 ? 45 : 0 },
      },
      yAxis: { type: 'value', axisLabel: { color: '#c9cde8' } },
      series: [
        {
          type: 'line',
          smooth: true,
          data: data.map((d) => d.value),
          areaStyle: {
            color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
              { offset: 0, color: 'rgba(110,231,183,.5)' },
              { offset: 1, color: 'rgba(110,231,183,0)' },
            ]),
          },
          lineStyle: { color: '#6ee7b7', width: 2 },
          itemStyle: { color: '#6ee7b7' },
        },
      ],
    })
    charts.push(c)
  }
}

function onResize() {
  charts.forEach((c) => c.resize())
}
window.addEventListener('resize', onResize)
onBeforeUnmount(() => {
  window.removeEventListener('resize', onResize)
  charts.forEach((c) => c.dispose())
})

const granOptions: { id: Gran; label: string }[] = [
  { id: 'day', label: '天' },
  { id: 'week', label: '周' },
  { id: 'month', label: '月' },
  { id: 'year', label: '年' },
]

function setGran(g: Gran) {
  gran.value = g
  load()
}

const kpi = computed(() => {
  // 工作时长只统计非休息帧；休息帧（熄屏/黑屏）单独计。
  const workFrames = frames.value.filter((f) => !f.rest)
  const restFrames = frames.value.filter((f) => f.rest)
  return {
    frames: frames.value.length,
    workMinutes: Math.round((workFrames.length * 20) / 60),
    restMinutes: Math.round((restFrames.length * 20) / 60),
    t10: t10.value.length,
    apps: new Set(frames.value.map((f) => f.app)).size,
  }
})
</script>

<template>
  <div>
    <div class="board-head">
      <h1 class="page" style="margin: 0">看板</h1>
      <div class="seg">
        <button
          v-for="g in granOptions"
          :key="g.id"
          :class="['seg-btn', { on: gran === g.id }]"
          @click="setGran(g.id)"
        >
          {{ g.label }}
        </button>
      </div>
    </div>

    <div v-if="errMsg" class="chip warn">{{ errMsg }}</div>

    <div class="kpi">
      <div class="glass card"><h3>帧数</h3><div class="big-num">{{ kpi.frames }}</div></div>
      <div class="glass card"><h3>工作</h3><div class="big-num">{{ kpi.workMinutes }} <small>min</small></div></div>
      <div class="glass card"><h3>休息</h3><div class="big-num">{{ kpi.restMinutes }} <small>min</small></div></div>
      <div class="glass card"><h3>10分钟片段</h3><div class="big-num">{{ kpi.t10 }}</div></div>
      <div class="glass card"><h3>应用数</h3><div class="big-num">{{ kpi.apps }}</div></div>
    </div>

    <div class="row">
      <div class="glass card chart">
        <h3>活动类型时间占比</h3>
        <div ref="elPie" class="cv"></div>
      </div>
      <div class="glass card chart">
        <h3>软件出现次数 Top 10</h3>
        <div ref="elApp" class="cv"></div>
      </div>
    </div>

    <div class="row">
      <div class="glass card chart">
        <h3>项目专注时长（分钟）</h3>
        <div ref="elProj" class="cv"></div>
      </div>
      <div class="glass card chart">
        <h3>帧量热力 · 小时 × 星期</h3>
        <div ref="elHeat" class="cv"></div>
      </div>
    </div>

    <div class="glass card chart">
      <h3>逐日帧量趋势</h3>
      <div ref="elTrend" class="cv"></div>
    </div>

    <div class="glass card" style="text-align:right; margin-top: 8px">
      <button class="btn primary" @click="load" :style="{ margin: '0 auto' }">重新加载</button>
    </div>
  </div>
</template>

<style scoped>
.board-head {
  display: flex;
  justify-content: space-between;
  /* align-items: center; */
  margin-bottom: 14px;
}
.seg {
  display: flex;
  gap: 4px;
  padding: 4px;
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.05);
  border: 1px solid rgba(255, 255, 255, 0.1);
}
.seg-btn {
  padding: 6px 16px;
  border-radius: 9px;
  border: none;
  background: transparent;
  color: #c9cde8;
  font-size: 13px;
  cursor: pointer;
}
.seg-btn.on {
  background: rgba(90, 140, 255, 0.3);
  color: #fff;
}
.kpi {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 14px;
  margin-bottom: 16px;
}
.big-num {
  font-size: 30px;
  font-weight: 800;
  background: linear-gradient(90deg, #7aa5ff, #c084fc);
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}
.big-num small {
  font-size: 14px;
  -webkit-text-fill-color: #8b90b5;
}
.chart {
  flex: 1;
  min-width: 300px;
}
.cv {
  width: 100%;
  height: 260px;
}
</style>
