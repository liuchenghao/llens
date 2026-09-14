<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { convertFileSrc } from '@tauri-apps/api/core'

type Frame = {
  time: string
  image: string
  extra_images?: string[]
  summary5: string[]
  activity?: string
  app?: string
  project?: string
}

const frames = ref<Frame[]>([])
const loading = ref(false)
const errMsg = ref('')
const dataRoot = ref('')

async function load() {
  loading.value = true
  errMsg.value = ''
  try {
    if (!dataRoot.value) dataRoot.value = await invoke<string>('data_root')
    const day = new Date()
    const d = `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, '0')}-${String(day.getDate()).padStart(2, '0')}`
    frames.value = await invoke<Frame[]>('list_day', { day: d })
  } catch (e: any) {
    errMsg.value = String(e)
  } finally {
    loading.value = false
  }
}

onMounted(load)
setInterval(load, 30000)

const byActivity = computed(() => {
  const m: Record<string, number> = {}
  for (const f of frames.value) {
    const k = f.activity || '未分类'
    m[k] = (m[k] || 0) + 1
  }
  return Object.entries(m).sort((a, b) => b[1] - a[1])
})

function imgSrc(rel: string) {
  const abs = rel.startsWith('/') ? rel : `${dataRoot.value}/${rel}`
  return convertFileSrc(abs)
}
</script>

<template>
  <div>
    <h1 class="page">今日概览</h1>
    <div class="row">
      <div class="glass card">
        <h3>今日帧数</h3>
        <div class="big-num">{{ frames.length }}</div>
        <div class="muted">每 20 秒一帧 · 自动记录</div>
      </div>
      <div class="glass card" style="flex: 1">
        <h3>活动分布</h3>
        <div v-if="byActivity.length">
          <span v-for="[k, v] in byActivity" :key="k" class="chip">{{ k }} × {{ v }}</span>
        </div>
        <div v-else class="muted">今天还没有记录</div>
      </div>
      <div class="glass card">
        <h3>最近</h3>
        <div v-if="frames.length" class="muted">
          {{ frames[frames.length - 1].time }} · {{ frames[frames.length - 1].app || '—' }}
        </div>
        <div v-else class="muted">—</div>
      </div>
    </div>

    <div class="glass card" v-if="errMsg">
      <h3>状态</h3>
      <div class="chip warn">截屏读取失败：{{ errMsg }}（请检查屏幕录制权限）</div>
    </div>

    <div class="glass card">
      <h3>时间轴 · 倒序</h3>
      <div v-if="loading && !frames.length" class="muted">加载中…</div>
      <div v-else-if="!frames.length" class="muted">暂无数据（刚启动？等待前几帧）</div>
      <div v-else class="timeline">
        <div v-for="(f, i) in [...frames].reverse()" :key="i" class="tl-item">
          <img v-if="f.image" :src="imgSrc(f.image)" class="thumb" alt="" />
          <div class="tl-body">
            <div class="tl-head">
              <span class="tl-time">{{ f.time.slice(11, 16) }}</span>
              <span v-if="f.app" class="chip">{{ f.app }}</span>
              <span v-if="f.activity" class="chip warn">{{ f.activity }}</span>
              <span v-if="f.project" class="chip">项目: {{ f.project }}</span>
            </div>
            <div class="tl-sent">
              <span v-for="(s, j) in f.summary5" :key="j">{{ s }}&nbsp;</span>
              <span v-if="!f.summary5.length" class="muted">（总结生成中或失败）</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.big-num {
  font-size: 40px;
  font-weight: 800;
  background: linear-gradient(90deg, #7aa5ff, #c084fc);
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}
.timeline {
  display: flex;
  flex-direction: column;
  gap: 12px;
  max-height: 52vh;
  overflow-y: auto;
}
.tl-item {
  display: flex;
  gap: 14px;
  padding: 10px;
  border-radius: 14px;
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.07);
}
.thumb {
  width: 110px;
  height: 70px;
  object-fit: cover;
  border-radius: 10px;
  flex-shrink: 0;
  filter: brightness(0.95);
}
.tl-head {
  display: flex;
  gap: 6px;
  align-items: center;
  margin-bottom: 6px;
  flex-wrap: wrap;
}
.tl-time {
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}
.tl-sent {
  font-size: 13px;
  color: #c9cde8;
  line-height: 1.5;
}
</style>
