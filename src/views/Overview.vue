<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, computed, watch, nextTick } from 'vue'
import { invoke } from '@tauri-apps/api/core'

type Frame = {
  time: string
  image: string
  extra_images?: string[]
  thumb?: string
  preview?: string
  extra_thumbs?: string[]
  extra_previews?: string[]
  summary5: string[]
  activity?: string
  app?: string
  project?: string
  rest?: boolean
}

const frames = ref<Frame[]>([])
const loading = ref(false)
const errMsg = ref('')
const dataRoot = ref('')
const retrying = ref<Set<string>>(new Set())
const previewFrame = ref<Frame | null>(null)

// 筛选条件（已应用——点「搜索」后生效）
const filterActivity = ref<string>('')
const filterApp = ref<string>('')
const filterProject = ref<string>('')
const filterKeyword = ref<string>('')

// 草稿筛选条件（下拉框/输入框绑定的中间值，点「搜索」才应用到 filterXxx）
const draftActivity = ref<string>('')
const draftApp = ref<string>('')
const draftProject = ref<string>('')
const draftKeyword = ref<string>('')

// 是否有任何待应用的草稿
const hasDraft = computed(() =>
  draftActivity.value !== filterActivity.value ||
  draftApp.value !== filterApp.value ||
  draftProject.value !== filterProject.value ||
  draftKeyword.value !== filterKeyword.value
)

function applySearch() {
  filterActivity.value = draftActivity.value
  filterApp.value = draftApp.value
  filterProject.value = draftProject.value
  filterKeyword.value = draftKeyword.value
  page.value = 1
}
function resetFilters() {
  filterActivity.value = ''
  filterApp.value = ''
  filterProject.value = ''
  filterKeyword.value = ''
  draftActivity.value = ''
  draftApp.value = ''
  draftProject.value = ''
  draftKeyword.value = ''
  page.value = 1
}

// 分页
const PAGE_SIZE = 20
const page = ref(1)
const totalPages = ref(1)

// 筛选后的全量结果（用于分页 + 统计），按时间倒序（最新在前）
const filtered = computed(() => {
  let list = frames.value
  if (filterActivity.value) list = list.filter(f => f.activity === filterActivity.value)
  if (filterApp.value) list = list.filter(f => f.app === filterApp.value)
  if (filterProject.value) list = list.filter(f => f.project === filterProject.value)
  if (filterKeyword.value.trim()) {
    const kw = filterKeyword.value.trim().toLowerCase()
    list = list.filter(f =>
      (f.summary5 || []).some(s => s.toLowerCase().includes(kw)) ||
      (f.app || '').toLowerCase().includes(kw) ||
      (f.project || '').toLowerCase().includes(kw)
    )
  }
  // 全量按时间倒序：最新帧在前，跨页顺序一致
  return [...list].sort((a, b) => b.time.localeCompare(a.time))
})
const pagedFrames = computed(() => {
  const total = filtered.value.length
  totalPages.value = Math.max(1, Math.ceil(total / PAGE_SIZE))
  if (page.value > totalPages.value) page.value = totalPages.value
  const start = (page.value - 1) * PAGE_SIZE
  // filtered 已倒序，直接切片即可，页内保持最新在前
  return filtered.value.slice(start, start + PAGE_SIZE)
})

// 可筛选的候选值（从全量帧中提取，去重）
function collectOpts(get: (f: Frame) => string | undefined): string[] {
  return [...new Set(frames.value.map(get).filter((s): s is string => !!s))].sort()
}
const activityOptions = computed<string[]>(() => collectOpts(f => f.activity))
const appOptions = computed<string[]>(() => collectOpts(f => f.app))
const projectOptions = computed<string[]>(() => collectOpts(f => f.project))

// 可搜索下拉框
const selectOpen = ref<'app' | 'project' | 'activity' | null>(null)
const selectQuery = ref('')
function openSelect(key: 'app' | 'project' | 'activity') {
  selectOpen.value = key
  selectQuery.value = ''
  nextTick(() => {
    const el = document.querySelector('.ss-input') as HTMLInputElement | null
    if (el) el.focus()
  })
}
function closeSelect() { selectOpen.value = null }
function pickApp(v: string) {
  draftApp.value = draftApp.value === v ? '' : v
  selectOpen.value = null
}
function pickProject(v: string) {
  draftProject.value = draftProject.value === v ? '' : v
  selectOpen.value = null
}
function pickActivity(v: string) {
  draftActivity.value = draftActivity.value === v ? '' : v
  selectOpen.value = null
}
function filteredAppOptions() {
  const q = selectQuery.value.trim().toLowerCase()
  const opts = appOptions.value
  return q ? opts.filter((o) => o.toLowerCase().includes(q)) : opts
}
function filteredProjectOptions() {
  const q = selectQuery.value.trim().toLowerCase()
  const opts = projectOptions.value
  return q ? opts.filter((o) => o.toLowerCase().includes(q)) : opts
}
function filteredActivityOptions() {
  const q = selectQuery.value.trim().toLowerCase()
  const opts = activityOptions.value
  return q ? opts.filter((o) => o.toLowerCase().includes(q)) : opts
}

// image cache: abs path -> data URL (sync read, async fill)
const imgUrls = ref<Record<string, string>>({})

function absOf(rel: string): string {
  return rel.startsWith('/') ? rel : `${dataRoot.value}/${rel}`
}
// Sync: returns cached URL or a 1×1 transparent placeholder.
function imgSrc(rel: string): string {
  return imgUrls.value[absOf(rel)] || 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7'
}

// 时间轴显示：优先用合成缩略图（<100KB，后端已横向拼接所有屏幕），没有则回退到原图路径
function thumbOf(f: Frame): string {
  return f.thumb || f.image
}
// 大图预览：优先用压缩预览图（<1MB），没有则回退到原图路径
function previewOf(f: Frame): string {
  return f.preview || f.image
}

// Fill the cache with base64 data URLs via Tauri command (works with any data root).
// Only preloads images for the currently visible page to avoid loading hundreds
// of data URLs at once when the day has many frames.
async function preloadImages() {
  const toLoad: string[] = []
  for (const f of pagedFrames.value) {
    // 时间轴：合成缩略图（单张，已包含所有屏幕）
    const t = thumbOf(f)
    if (t && !imgUrls.value[absOf(t)]) toLoad.push(t)
    // 预览弹窗：主屏预览图
    const p = previewOf(f)
    if (p && p !== t && !imgUrls.value[absOf(p)]) toLoad.push(p)
    // 预览弹窗：各副屏独立预览图
    for (const ep of f.extra_previews || []) {
      if (ep && !imgUrls.value[absOf(ep)]) toLoad.push(ep)
    }
    // 兼容旧数据：extra_images（原图路径）
    for (const ei of f.extra_images || []) {
      if (ei && !imgUrls.value[absOf(ei)]) toLoad.push(ei)
    }
  }
  if (!toLoad.length) return
  for (const rel of toLoad) {
    try {
      const url = await invoke<string>('read_image_as_data_url', { path: absOf(rel) })
      imgUrls.value[absOf(rel)] = url
    } catch { /* placeholder remains */ }
  }
  imgUrls.value = { ...imgUrls.value } // trigger reactivity
}

function openPreview(f: Frame) { previewFrame.value = f }

// 预览弹窗的多屏大图列表：优先用压缩预览图（extra_previews），
// 回退到原图（extra_images）；取两者最大长度，过滤掉无对应文件的空项，
// 避免渲染 1×1 透明 placeholder 让用户误以为“只有一张”。
function previewExtraImages(f: Frame): string[] {
  const eps = f.extra_previews || []
  const eis = f.extra_images || []
  const n = Math.max(eps.length, eis.length)
  const out: string[] = []
  for (let i = 0; i < n; i++) {
    const src = eps[i] || eis[i] || ''
    if (src) out.push(src)
  }
  return out
}
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
    // 默认今天；被 jump 设过则用指定日期
    const d = selDay.value
    frames.value = await invoke<Frame[]>('list_day', { day: d })
    // 重新拉取帧后，把草稿同步为当前已应用的筛选，避免下拉框显示过期的选中值
    draftActivity.value = filterActivity.value
    draftApp.value = filterApp.value
    draftProject.value = filterProject.value
    draftKeyword.value = filterKeyword.value
  } catch (e: any) {
    errMsg.value = String(e)
  } finally {
    loading.value = false
  }
  await preloadImages()
}

// 「更新」按钮：刷新帧数据并重新提取下拉框候选值
function refreshOptions() {
  load()
}

// 当前展示日期（默认今天；问答跳转时切换）
function todayStr() {
  const day = new Date()
  return `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, '0')}-${String(day.getDate()).padStart(2, '0')}`
}
const selDay = ref(todayStr())

// 问答跳转：监听 window 的 llens_jump 事件，加载指定日期
let jumpHandler: ((e: Event) => void) | null = null
onBeforeUnmount(() => {
  if (jumpHandler) window.removeEventListener('llens_jump', jumpHandler)
})
// 顶部日期切换控件
function changeDay(v: string) {
  if (!v) return
  selDay.value = v
  load()
}

onMounted(() => {
  load()
  jumpHandler = (e: Event) => {
    const detail = (e as CustomEvent).detail
    if (detail?.time) {
      selDay.value = detail.time
      load()
    }
  }
  window.addEventListener('llens_jump', jumpHandler)
})
setInterval(load, 30000)

// 翻页或已应用筛选变化后，确保新可见页的图已加载（草稿变化不触发）
watch([page, filterActivity, filterApp, filterProject, filterKeyword], () => {
  preloadImages().catch(() => {})
})

const byActivity = computed(() => {
  const m: Record<string, number> = {}
  for (const f of frames.value) {
    const k = f.activity || '未分类'
    m[k] = (m[k] || 0) + 1
  }
  return Object.entries(m).sort((a, b) => b[1] - a[1])
})

// 当前筛选命中的时间范围（用于时间轴副标题）
const timeRange = computed(() => {
  const l = filtered.value
  if (!l.length) return ''
  return `${l[0].time.slice(11, 16)} – ${l[l.length - 1].time.slice(11, 16)}`
})
</script><template>
  <div class="overview-root">
    <h1 class="page">概览</h1>
    <div class="row">
      <div class="glass card">
        <h3>{{ selDay === todayStr() ? '今日' : '当日' }}帧数</h3>
        <div class="big-num">{{ filtered.length }}</div>
        <div class="muted">每 20 秒一帧 · 自动记录</div>
        <div class="day-pick">
          <label class="day-pick-label">
            <span class="muted small">查看日期</span>
            <input
              type="date"
              :value="selDay"
              @change="changeDay(($event as any).target.value)"
              class="date-input f-input"
            />
          </label>
          <button class="btn small" @click="selDay = todayStr(); load()">回到今天</button>
        </div>
      </div>
      <div class="glass card" style="flex: 1">
        <h3>活动分布</h3>
        <div v-if="byActivity.length">
          <span
            v-for="[k, v] in byActivity" :key="k" class="chip"
            :class="{ active: draftActivity === k }"
            @click="draftActivity = draftActivity === k ? '' : k"
          >{{ k }} × {{ v }}</span>
        </div>
        <div v-else class="muted">今天还没有记录</div>
      </div>
      <div class="glass card">
        <h3>最近</h3>
        <div v-if="filtered.length" class="muted">
          {{ filtered[0].time }} · {{ filtered[0].app || '—' }}
        </div>
        <div v-else class="muted">—</div>
      </div>
    </div>

    <div class="glass card timeline-card">
      <div class="tl-top">
        <h3>时间轴 · 倒序 <span v-if="filtered.length" class="muted small">（{{ timeRange }}）</span></h3>
        <div class="filters">
          <!-- 关键词搜索（最前，宽 200px） -->
          <input
            v-model="draftKeyword"
            placeholder="关键词搜索摘要"
            class="f-input kw-input"
          />
          <!-- 应用下拉框 -->
          <div class="ss" @click.stop="selectOpen === 'app' ? closeSelect() : openSelect('app')">
            <button class="f-input ss-btn" :class="{ active: !!draftApp }" type="button">
              <span class="ss-label">{{ draftApp || '应用' }}</span>
              <span class="ss-caret">▾</span>
            </button>
            <div v-if="selectOpen === 'app'" class="ss-drop" @click.stop>
              <input v-model="selectQuery" class="ss-input" placeholder="搜索应用…" @click.stop />
              <div class="ss-list">
                <button v-for="o in filteredAppOptions()" :key="o" class="ss-opt" :class="{ sel: o === draftApp }" @click.stop="pickApp(o)">{{ o }}</button>
                <div v-if="!filteredAppOptions().length" class="ss-empty">无匹配</div>
              </div>
            </div>
          </div>
          <!-- 项目下拉框 -->
          <div class="ss" @click.stop="selectOpen === 'project' ? closeSelect() : openSelect('project')">
            <button class="f-input ss-btn" :class="{ active: !!draftProject }" type="button">
              <span class="ss-label">{{ draftProject || '项目' }}</span>
              <span class="ss-caret">▾</span>
            </button>
            <div v-if="selectOpen === 'project'" class="ss-drop" @click.stop>
              <input v-model="selectQuery" class="ss-input" placeholder="搜索项目…" @click.stop />
              <div class="ss-list">
                <button v-for="o in filteredProjectOptions()" :key="o" class="ss-opt" :class="{ sel: o === draftProject }" @click.stop="pickProject(o)">{{ o }}</button>
                <div v-if="!filteredProjectOptions().length" class="ss-empty">无匹配</div>
              </div>
            </div>
          </div>
          <!-- 活动下拉框 -->
          <div class="ss" @click.stop="selectOpen === 'activity' ? closeSelect() : openSelect('activity')">
            <button class="f-input ss-btn" :class="{ active: !!draftActivity }" type="button">
              <span class="ss-label">{{ draftActivity || '活动' }}</span>
              <span class="ss-caret">▾</span>
            </button>
            <div v-if="selectOpen === 'activity'" class="ss-drop" @click.stop>
              <input v-model="selectQuery" class="ss-input" placeholder="搜索活动…" @click.stop />
              <div class="ss-list">
                <button v-for="o in filteredActivityOptions()" :key="o" class="ss-opt" :class="{ sel: o === draftActivity }" @click.stop="pickActivity(o)">{{ o }}</button>
                <div v-if="!filteredActivityOptions().length" class="ss-empty">无匹配</div>
              </div>
            </div>
          </div>
          <!-- 操作按钮 -->
          <button class="btn primary" @click="applySearch" :disabled="!hasDraft">搜索</button>
          <button class="btn" @click="refreshOptions" :disabled="loading">更新</button>
          <button class="btn" @click="resetFilters" :disabled="!hasDraft && !filterActivity && !filterApp && !filterProject && !filterKeyword">清除</button>
        </div>
      </div>
      <div v-if="errMsg" class="status-row">
        <span class="chip warn">状态</span>
        <span class="chip warn">截屏读取失败：{{ errMsg }}（请检查屏幕录制权限）</span>
      </div>
      <div v-if="loading && !frames.length" class="muted">加载中…</div>
      <div v-else-if="!filtered.length" class="muted">暂无数据{{ frames.length ? '（当前筛选无命中）' : '（刚启动？等待前几帧）' }}</div>
      <div v-else class="timeline">
        <div v-for="(f) in pagedFrames" :key="f.time" class="tl-item" :class="{ rest: f.rest }">
          <img
            v-if="!f.rest && thumbOf(f)"
            :src="imgSrc(thumbOf(f))"
            class="thumb"
            alt=""
            @click="openPreview(f)"
          />
          <div v-else-if="f.rest" class="rest-badge" title="屏幕熄灭/黑屏，计为休息，未生成截图">
            <span class="rest-icon">☾</span>
            <span>休息</span>
          </div>
          <div class="tl-body">
            <div class="tl-head">
              <span class="tl-time">{{ f.time.slice(11, 16) }}</span>
              <template v-if="!f.rest">
                <span v-if="f.app" class="chip">{{ f.app }}</span>
                <span v-if="f.activity" class="chip warn">{{ f.activity }}</span>
                <span v-if="f.project" class="chip">项目: {{ f.project }}</span>
              </template>
              <span v-else class="chip warn">休息</span>
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
        <!-- 分页控件：固定在时间轴右下角 -->
      <div v-if="totalPages > 1" class="pager-fixed">
        <button class="btn" :disabled="page <= 1" @click="page--">‹ 上一页</button>
        <span class="muted">第 {{ page }} / {{ totalPages }} 页</span>
        <button class="btn" :disabled="page >= totalPages" @click="page++">下一页 ›</button>
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
              v-if="previewOf(previewFrame)"
              :src="imgSrc(previewOf(previewFrame))"
              class="preview-img"
              alt="主屏"
            />
            <img
              v-for="(ep, i) in previewExtraImages(previewFrame)"
              :key="i"
              :src="imgSrc(ep)"
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
.day-pick {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 10px;
  white-space: nowrap;
}
.day-pick-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.date-input {
  font-size: 12px;
  padding: 4px 8px;
  border-radius: 8px;
  border: 1px solid rgba(255, 255, 255, 0.12);
  background: rgba(255, 255, 255, 0.06);
  color: #e6e9ff;
  outline: none;
  cursor: pointer;
  color-scheme: dark;
}
.date-input:hover {
  border-color: rgba(143, 214, 255, 0.4);
  background: rgba(143, 214, 255, 0.08);
}
.date-input:focus {
  border-color: rgba(143, 214, 255, 0.6);
}
/* outer flex column: fills remaining content area, bottom 30px */
.overview-root {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  
}
.status-row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  padding: 8px 12px;
  margin: 8px 0;
  border-radius: 10px;
  background: rgba(255, 196, 111, 0.08);
  border: 1px solid rgba(255, 196, 111, 0.25);
}
.overview-root > .row {
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
  position: relative;
}
.timeline-card {
  position: relative;
}
.timeline-card .timeline {
  position: relative;
}
.pager-fixed {
  position: sticky;
  bottom: 0;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 4px 4px;
  flex-shrink: 0;
  background: rgba(10,12,26,0.85);
  backdrop-filter: blur(8px);
  justify-content: flex-end;
  border-radius: 8px;
  z-index: 10;
}
/* 分页按钮：与 .f-input 输入框同高度 */
.pager-fixed .btn {
  padding: 4px 12px;
  border-radius: 8px;
  font-size: 12px;
  line-height: 18px;
  height: auto;
  background: rgba(255,255,255,0.06);
  border: 1px solid rgba(255,255,255,0.15);
  color: #e6e9ff;
  white-space: nowrap;
  transition: all 0.15s;
}
.pager-fixed .btn:hover:not(:disabled) { background: rgba(255,255,255,0.15); }
.pager-fixed .btn:disabled { opacity: 0.45; cursor: not-allowed; }

/* 可搜索下拉框 */
.ss { position: relative; flex: 1;}
.ss-btn {
  display: flex;
  align-items: center;
  justify-content: space-between;
  cursor: pointer;
  gap: 6px;
  padding-right: 8px;
  width: 100%;
}
.ss-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
}
.ss-caret { font-size: 10px; opacity: 0.7; }
.ss-btn.active { border-color: rgba(90,140,255,0.6); }
.ss-drop {
  position: absolute;
  top: 100%;
  left: 0;
  z-index: 100;
  width: 200px;
  background: rgba(15,17,35,0.98);
  border: 1px solid rgba(255,255,255,0.15);
  border-radius: 10px;
  margin-top: 4px;
  overflow: hidden;
  box-shadow: 0 8px 24px rgba(0,0,0,0.45);
}
.ss-input {
  width: 100%;
  box-sizing: border-box;
  background: transparent;
  border: none;
  border-bottom: 1px solid rgba(255,255,255,0.08);
  padding: 7px 10px;
  font-size: 12px;
  color: #e6e9ff;
  outline: none;
}
.ss-input::placeholder { color: #6a70a0; }
.ss-list {
  max-height: 200px;
  overflow-y: auto;
}
.ss-opt {
  display: block;
  width: 100%;
  text-align: left;
  background: transparent;
  border: none;
  padding: 7px 10px;
  font-size: 12px;
  color: #c9cde8;
  cursor: pointer;
}
.ss-opt:hover { background: rgba(255,255,255,0.08); }
.ss-opt.sel {
  color: #8fd6ff;
  font-weight: 700;
  background: rgba(90,140,255,0.15);
}
.ss-empty {
  padding: 8px 10px;
  font-size: 11px;
  color: #6a70a0;
}

/* 关键词搜索输入框（宽 200px） */
.kw-input {
  width: 200px;
  background: rgba(255,255,255,0.06);
  border: 1px solid rgba(255,255,255,0.15);
  border-radius: 8px;
  padding: 4px 10px;
  font-size: 12px;
  color: #e6e9ff;
  outline: none;
  transition: all 0.15s;
}
.kw-input:focus { border-color: rgba(90,140,255,0.6); }
.kw-input::placeholder { color: #6a70a0; }

/* 操作按钮组 */
.filters {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  align-items: center;
  flex: 1;
}
.btn.primary {
  background: rgba(90,140,255,0.35);
  border-color: rgba(90,140,255,0.7);
  color: #eef2ff;
  font-weight: 700;
}
.btn.primary:disabled { opacity: 0.5; cursor: not-allowed; }

/* 筛选区按钮：高度与 .f-input 输入框一致（不再用全局 .btn 的大 padding） */
.filters .btn {
  padding: 4px 12px;
  border-radius: 8px;
  font-size: 12px;
  line-height: 18px;
  height: auto;
  background: rgba(255,255,255,0.06);
  border: 1px solid rgba(255,255,255,0.15);
  color: #e6e9ff;
  white-space: nowrap;
  transition: all 0.15s;
}
.filters .btn:hover:not(:disabled) { background: rgba(255,255,255,0.15); }
.filters .btn:disabled { opacity: 0.45; cursor: not-allowed; }
.filters .btn.primary {
  background: rgba(90,140,255,0.35);
  border-color: rgba(90,140,255,0.7);
  color: #eef2ff;
  font-weight: 700;
}

.tl-top {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
  flex-shrink: 0;
  display: flex;
}
.tl-top h3 { margin: 0; }
.small { font-size: 12px; }

/* .filters 已在上方「操作按钮组」定义，此处不再重复 */
.f-input {
  background: rgba(255,255,255,0.06);
  border: 1px solid rgba(255,255,255,0.15);
  border-radius: 8px;
  padding: 4px 10px;
  font-size: 12px;
  color: #e6e9ff;
  outline: none;
  transition: all 0.15s;
  flex: 1;
}
.f-input:focus { border-color: rgba(90,140,255,0.6); }
.f-input::placeholder { color: #6a70a0; }
.chip.active {
  background: rgba(90,140,255,0.35);
  border-color: rgba(90,140,255,0.7);
  color: #eef2ff;
  cursor: pointer;
}
.chip { cursor: default; }
.pager {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 0 2px;
  flex-shrink: 0;
}
.tl-item {
  display: flex;
  gap: 14px;
  padding: 10px;
  border-radius: 14px;
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.07);
}
.rest-badge {
  width: 120px;
  height: 66px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 2px;
  border-radius: 10px;
  background: rgba(255,255,255,0.05);
  border: 1px solid rgba(255,255,255,0.08);
  color: #8b90b5;
  font-size: 12px;
  letter-spacing: 1px;
}
.rest-icon { font-size: 20px; opacity: .8; }

.thumb {
  width: auto;
  max-width: 220px;
  height: 70px;
  object-fit: contain;
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
  gap: 12px;
  padding: 12px 16px;
  overflow: auto;
  min-height: 0;
  justify-content: center;
  align-items: center;
}
.preview-img {
  height: auto;
  max-height: 40vh;
  width: auto;
  max-width: calc(50% - 12px);
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
