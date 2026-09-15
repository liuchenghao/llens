<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'

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
const retrying = ref<Set<string>>(new Set())
const previewFrame = ref<Frame | null>(null)

// image cache: abs path -> data URL (sync read, async fill)
const imgUrls = ref<Record<string, string>>({})

function absOf(rel: string): string {
  return rel.startsWith('/') ? rel : `${dataRoot.value}/${rel}`
}
// Sync: returns cached URL or a 1×1 transparent placeholder.
function imgSrc(rel: string): string {
  return imgUrls.value[absOf(rel)] || 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7'
}
// Fill the cache with base64 data URLs via Tauri command (works with any data root).
async function preloadImages() {
  const toLoad: string[] = []
  for (const f of frames.value) {
    if (f.image && !imgUrls.value[absOf(f.image)]) toLoad.push(f.image)
    for (const extra of f.extra_images || []) {
      if (!imgUrls.value[absOf(extra)]) toLoad.push(extra)
    }
  }
  for (const rel of toLoad) {
    try {
      const url = await invoke<string>('read_image_as_data_url', { path: absOf(rel) })
      imgUrls.value[absOf(rel)] = url
    } catch { /* placeholder remains */ }
  }
  imgUrls.value = { ...imgUrls.value } // trigger reactivity
}

function openPreview(f: Frame) { previewFrame.value = f }
function closePreview() { previewFrame.value = null }

async function retrySummary(f: Frame) {
  if (retrying.value.has(f.time)) return
  retrying.value.add(f.time)
  try {
    const day = f.time.slice(0, 10)
    const updated = await invoke<Frame>('retry_frame_summary', { day, time: f.time })
    const idx = frames.value.findIndex((x) => x.time === f.time)
    if (idx >= 0) frames.value[idx] = updated
    await preloadImages()
  } catch (e: any) {
    errMsg.value = `重新生成失败：${String(e)}`
  } finally {
    retrying.value.delete(f.time)
  }
}

async function load() {
  loading.value = true
  errMsg.value = ''
  try {
    dataRoot.value = await invoke<string>('data_root')
    const day = new Date()
    const d = `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, '0')}-${String(day.getDate()).padStart(2, '0')}`
    frames.value = await invoke<Frame[]>('list_day', { day: d })
    await preloadImages()
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
</script>

<template>
  <div class="overview-root">
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

    <div class="glass card err-card" v-if="errMsg">
      <h3>状态</h3>
      <div class="chip warn">截屏读取失败：{{ errMsg }}（请检查屏幕录制权限）</div>
    </div>

    <div class="glass card timeline-card">
      <h3>时间轴 · 倒序</h3>
      <div v-if="loading && !frames.length" class="muted">加载中…</div>
      <div v-else-if="!frames.length" class="muted">暂无数据（刚启动？等待前几帧）</div>
      <div v-else class="timeline">
        <div v-for="(f, i) in [...frames].reverse()" :key="i" class="tl-item">
          <img
            v-if="f.image"
            :src="imgSrc(f.image)"
            class="thumb"
            alt=""
            @click="openPreview(f)"
          />
          <div class="tl-body">
            <div class="tl-head">
              <span class="tl-time">{{ f.time.slice(11, 16) }}</span>
              <span v-if="f.app" class="chip">{{ f.app }}</span>
              <span v-if="f.activity" class="chip warn">{{ f.activity }}</span>
              <span v-if="f.project" class="chip">项目: {{ f.project }}</span>
            </div>
            <div class="tl-sent">
              <span v-for="(s, j) in f.summary5" :key="j">{{ s }}&nbsp;</span>
              <template v-if="!f.summary5.length">
                <button
                  :class="['retry-btn', { busy: retrying.has(f.time) }]"
                  :disabled="retrying.has(f.time)"
                  @click="retrySummary(f)"
                >
                  {{ retrying.has(f.time) ? '生成中…' : '↻ 重新生成' }}
                </button>
                <span class="muted retry-hint">（总结生成失败，可重试）</span>
              </template>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- multi-screen image preview modal -->
    <Teleport to="body">
      <div v-if="previewFrame" class="preview-overlay" @click.self="closePreview">
        <div class="preview-modal glass">
          <div class="preview-head">
            <span class="preview-time">{{ previewFrame.time.slice(11, 16) }}</span>
            <span v-if="previewFrame.app" class="chip">{{ previewFrame.app }}</span>
            <span v-if="previewFrame.activity" class="chip warn">{{ previewFrame.activity }}</span>
            <button class="preview-close" @click="closePreview">×</button>
          </div>
          <div class="preview-images">
            <img
              :src="imgSrc(previewFrame.image)"
              class="preview-img"
              alt="主屏"
            />
            <img
              v-for="(extra, i) in (previewFrame.extra_images || [])"
              :key="i"
              :src="imgSrc(extra)"
              class="preview-img"
              :alt="`屏幕 ${i + 2}`"
            />
          </div>
          <div v-if="previewFrame.summary5.length" class="preview-summary">
            <span v-for="(s, j) in previewFrame.summary5" :key="j">{{ s }}&nbsp;</span>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
/* outer flex column: fills remaining content area, bottom 30px */
.overview-root {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  
}
.overview-root > .row {
  flex-shrink: 0;
}
.overview-root > .err-card {
  flex-shrink: 0;
}
.overview-root > .timeline-card {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
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
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  padding-right: 4px;
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
  cursor: pointer;
  transition: filter 0.15s, transform 0.15s;
}
.thumb:hover {
  filter: brightness(1.08);
  transform: scale(1.04);
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
.retry-btn {
  padding: 3px 10px;
  border-radius: 8px;
  border: 1px solid rgba(255, 176, 64, 0.5);
  background: rgba(255, 176, 64, 0.12);
  color: #ffcf94;
  font-size: 12px;
  cursor: pointer;
  margin-right: 8px;
  transition: all 0.2s;
}
.retry-btn:hover:not(:disabled) {
  background: rgba(255, 176, 64, 0.25);
}
.retry-btn:disabled {
  opacity: 0.5;
  cursor: wait;
}
.retry-hint {
  font-size: 11px;
}

/* image preview modal */
.preview-overlay {
  position: fixed;
  inset: 0;
  z-index: 1000;
  background: rgba(0, 0, 0, 0.72);
  display: flex;
  align-items: center;
  justify-content: center;
  backdrop-filter: blur(6px);
  -webkit-backdrop-filter: blur(6px);
  animation: fadeIn 0.18s ease;
}
@keyframes fadeIn {
  from { opacity: 0; }
  to   { opacity: 1; }
}
.preview-modal {
  max-width: 90vw;
  max-height: 85vh;
  width: auto;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  border-radius: 16px;
}
.preview-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 16px 8px;
  flex-shrink: 0;
}
.preview-time {
  font-weight: 700;
  font-size: 14px;
  font-variant-numeric: tabular-nums;
  color: #eef0ff;
}
.preview-close {
  margin-left: auto;
  background: transparent;
  border: 1px solid rgba(255,255,255,0.2);
  border-radius: 8px;
  color: #c9cde8;
  font-size: 18px;
  line-height: 1;
  padding: 4px 10px;
  cursor: pointer;
  transition: all 0.15s;
}
.preview-close:hover {
  background: rgba(255,255,255,0.1);
  color: #fff;
}
.preview-images {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  gap: 10px;
  padding: 8px 16px;
  overflow: auto;
  min-height: 0;
  align-items: flex-start;
}
.preview-img {
  height: 40vh;
  width: auto;
  object-fit: contain;
  border-radius: 10px;
  border: 1px solid rgba(255,255,255,0.1);
  background: #0a0c1a;
  flex-shrink: 0;
}
.preview-summary {
  padding: 10px 16px 14px;
  font-size: 13px;
  color: #c9cde8;
  line-height: 1.6;
  border-top: 1px solid rgba(255,255,255,0.08);
  flex-shrink: 0;
}
</style>
