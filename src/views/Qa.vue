<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'

type Plan = {
  keywords: string[]
  layer: string
  days: number
  need_more: boolean
  limit?: number
}
type Hit = { layer: string; time: string; text: string; source: string }
type QaAnswer = { text: string; citations: number[] }
// 多轮问答历史（追问上下文）
type ConvTurn = { question: string; answer: string }

const question = ref('')
const rounds = ref<{ plan: Plan; hits: Hit[] }[]>([])
const answer = ref<QaAnswer>({ text: '', citations: [] })
const busy = ref(false)
const err = ref('')
const showTrajectory = ref(false) // 检索轨迹默认折叠
// 会话历史：支持多轮追问（最多保留最近 3 轮）
const history = ref<ConvTurn[]>([])

// 单轮命中上限（可调，1-100）。默认 20。
const hitLimit = ref<number>(20)
function effectiveLimit(): number {
  const v = Number(hitLimit.value)
  if (!Number.isFinite(v) || v < 1) return 20
  return Math.min(Math.max(v, 1), 100)
}

const historyParam = () => history.value.slice(-3)

// 命中引用：把 answer.citations（0-based hit 下标）映射回 hit 对象
const allHits = computed(() => rounds.value.flatMap((r) => r.hits))
const citedHits = computed(() =>
  answer.value.citations.map((i) => allHits.value[i]).filter(Boolean)
)

// 预设问题快捷入口
const presets = [
  '今天主要在处理什么项目？',
  '最近一周主要工作时间分布？',
  '哪些天休息/离开较多？',
  '昨天下午在做什么？',
]
function pickPreset(q: string) {
  question.value = q
}

// 点击来源引用：在问答页内联预览该命中帧的截图；日记类则跳转日记页
const previewHit = ref<Hit | null>(null)
const previewCard = ref<HTMLElement | null>(null)
const previewImg = ref('')
const previewBusy = ref(false)
const dataRoot = ref('')

// 根据 hit.time 拉取当天帧，找到匹配帧后读取其预览图（主屏 + 各副屏）
const previewExtras = ref<string[]>([])

async function showPreview(h: Hit) {
  if (h.layer === 'diary') {
    // 日记来源：直接跳转到日记页对应日期，不做内联预览
    previewHit.value = null
    try {
      await invoke('qa_jump', { layer: h.layer, time: h.time })
    } catch { /* ignore */ }
    return
  }
  // 帧/切片来源：在问答页原地显示预览信息
  previewHit.value = h
  previewImg.value = ''
  previewExtras.value = []
  previewBusy.value = true
  // 渲染后滚动到预览卡片，让点击有可见反馈
  requestAnimationFrame(() => {
    previewCard.value?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  })
  type F = { time: string; extra_previews?: string[]; extra_images?: string[]; preview?: string; image?: string; thumb?: string }
  const absOf = (rel: string) => (rel.startsWith('/') ? rel : `${dataRoot.value}/${rel}`)
  const readImg = async (rel: string): Promise<string | null> => {
    if (!rel) return null
    try { return await invoke<string>('read_image_as_data_url', { path: absOf(rel) }) } catch { return null }
  }
  try {
    if (!dataRoot.value) dataRoot.value = await invoke<string>('data_root')
    // 拉当天帧；frame 层按时间匹配，t10 层按切片起点找最接近的帧
    const day = h.time.slice(0, 10)
    const frames = await invoke<F[]>('list_day', { day })
    let match: F | undefined
    // 主屏图相对路径：frame 层可从 h.source 去掉 "frame:" 前缀直接取；t10 层需从匹配帧取
    let mainRel = ''
    if (h.layer === 'frame') {
      match = frames.find((f) => f.time === h.time) || frames.find((f) => f.time.startsWith(h.time))
      // h.source 形如 "frame:<相对路径>"（可能为空），取冒号后部分
      mainRel = h.source.startsWith('frame:') ? h.source.slice(6) : h.source
      if (match) mainRel = match.preview || match.thumb || mainRel || ''
    } else {
      // t10：h.time 是 10 分钟对齐起点，取当天帧里时间 >= 起点的第一个帧（或最接近的）
      const t0 = h.time
      match = frames.find((f) => f.time === t0) ||
        frames.find((f) => f.time.startsWith(t0)) ||
        frames.find((f) => f.time >= t0) ||
        (frames.length ? frames[frames.length - 1] : undefined)
      if (match) mainRel = match.preview || match.thumb || match.image || ''
    }
    if (mainRel) {
      const p = await readImg(mainRel)
      if (p) previewImg.value = p
    }
    // 副屏预览图（若有）
    if (match) {
      const rels: string[] = []
      for (const p of match.extra_previews || []) rels.push(p)
      if (!rels.length) for (const p of match.extra_images || []) rels.push(p)
      for (const rel of rels.slice(0, 3)) {
        const a = await readImg(rel)
        if (a) previewExtras.value.push(a)
      }
    }
  } catch (e: any) {
    // 图读不到也不让卡片消失：保留 previewHit，仅把错误写进文本
    previewHit.value = { ...h, text: `${h.text}\n（预览图加载失败：${String(e).slice(0, 60)}）` }
  } finally {
    previewBusy.value = false
  }
}

function closePreview() {
  previewHit.value = null
  previewImg.value = ''
  previewExtras.value = []
}

// 检索轨迹每轮默认只显示前 8 条，可展开看全部
const expanded = ref<Record<number, boolean>>({})
function visibleHits(idx: number, hits: Hit[]) {
  return expanded.value[idx] ? hits : hits.slice(0, 8)
}
onMounted(async () => {
  try { dataRoot.value = await invoke<string>('data_root') } catch { /* ignore */ }
})

// Agentic flow, hard limits: <= 3 search rounds. 命中上限由 hitLimit 动态控制。
async function ask() {
  if (!question.value.trim() || busy.value) return
  busy.value = true
  err.value = ''
  answer.value = { text: '', citations: [] }
  rounds.value = []
  showTrajectory.value = false
  const limit = effectiveLimit()

  try {
    // round 1: model plans the search (with history context for follow-ups)
    let plan = await invoke<Plan>('qa_plan', { question: question.value, history: historyParam() })
    let hits = await invoke<Hit[]>('qa_search', { plan, limit })
    rounds.value.push({ plan, hits })

    // round 2: model may refine once (only if it still wants more AND we have budget)
    if (plan.need_more) {
      const plan2 = await invoke<Plan>('qa_refine', {
        question: question.value,
        prevHits: hits,
        history: historyParam(),
      })
      if (plan2.keywords.length && plan2.days > 0) {
        const hits2 = await invoke<Hit[]>('qa_search', { plan: plan2, limit })
        rounds.value.push({ plan: plan2, hits: hits2 })
        hits = [...hits, ...hits2]
      }
    }

    // final: answer from accumulated context (with citations + history)
    const res = await invoke<QaAnswer>('qa_answer', {
      question: question.value,
      hits,
      history: historyParam(),
    })
    answer.value = res

    // 把本轮问答加入历史（支持下一轮追问）
    history.value.push({ question: question.value.trim(), answer: res.text })
    if (history.value.length > 6) history.value = history.value.slice(-3)
  } catch (e: any) {
    err.value = String(e)
  } finally {
    busy.value = false
  }
}

function newConversation() {
  history.value = []
  question.value = ''
  answer.value = { text: '', citations: [] }
  rounds.value = []
}
</script>

<template>
  <div>
    <h1 class="page">问答</h1>

    <div class="glass card">
      <textarea
        v-model="question"
        rows="2"
        placeholder="例如：昨天下午主要在处理什么项目？最近一周娱乐占比多少？"
      ></textarea>
      <div class="presets">
        <button
          v-for="p in presets"
          :key="p"
          class="preset"
          :disabled="busy"
          @click="pickPreset(p)"
        >
          {{ p }}
        </button>
      </div>
      <div style="display: flex; gap: 10px; margin-top: 10px; align-items: center; flex-wrap: wrap">
        <button class="btn primary" @click="ask" :disabled="busy || !question.trim()">
          {{ busy ? '检索中…' : (history.length ? '追问' : '提问') }}
        </button>
        <button v-if="history.length" class="btn" @click="newConversation">新对话</button>
        <label class="limit-label">
          <span class="muted">单轮命中上限</span>
          <input
            type="number"
            min="1"
            max="100"
            :value="hitLimit"
            @change="hitLimit = Number(($event as any).target.value)"
            class="limit-input"
            :disabled="busy"
          />
        </label>
        <span class="muted" style="font-size: 11px">
          agentic 检索 · 最多 3 轮 · 单轮 ≤ {{ effectiveLimit() }} 条 × 200 字
        </span>
      </div>
    </div>

    <div v-if="err" class="glass card"><div class="chip warn">出错：{{ err }}</div></div>

    <!-- 可点来源引用：回答中 [n] 对应的 hit -->
    <div v-if="answer.text" class="glass card">
      <h3>回答</h3>
      <p class="answer">{{ answer.text }}</p>
      <div v-if="citedHits.length" class="citations">
        <span class="muted" style="font-size: 11px">来源：</span>
        <button
          v-for="(h, i) in citedHits"
          :key="i"
          class="cite"
          @click="showPreview(h)"
          :title="`layer=${h.layer} time=${h.time}`"
        >
          [{{ i + 1 }}] {{ h.layer === 'diary' ? '日记' : '概览' }} · {{ h.time.slice(0, 10) }}
        </button>
      </div>
    </div>

    <!-- 内联预览：点击来源引用后显示该命中帧的截图 + 上下文 -->
    <div v-if="previewHit" ref="previewCard" class="glass card">
      <div class="preview-head">
        <h3>来源预览</h3>
        <button class="preview-close" @click="closePreview">✕</button>
      </div>
      <div class="preview-meta">
        <code>{{ previewHit.time }}</code>
        <span class="chip">{{ previewHit.layer }}</span>
        <span class="muted">{{ previewHit.source }}</span>
      </div>
      <div v-if="previewBusy" class="muted">加载预览图…</div>
      <div v-else class="preview-imgs">
        <img v-if="previewImg" :src="previewImg" class="preview-img" />
        <img v-for="(ep, i) in previewExtras" :key="i" :src="ep" class="preview-img" />
        <div v-if="!previewImg && !previewExtras.length" class="muted">无可用预览图（该来源可能已清理或为文本层）</div>
      </div>
      <p class="preview-text">{{ previewHit.text }}</p>
    </div>

    <!-- 检索轨迹（默认折叠） -->
    <div v-if="rounds.length" class="glass card">
      <h3 class="traj-head" @click="showTrajectory = !showTrajectory">
        <span>检索轨迹</span>
        <span class="muted" style="font-size: 11px">{{ showTrajectory ? '▾' : '▸' }}</span>
      </h3>
      <div v-if="showTrajectory">
        <div v-for="(r, i) in rounds" :key="i" class="round">
          <div class="round-head">
            <span class="chip">第 {{ i + 1 }} 轮</span>
            <span class="chip warn">{{ r.plan.layer }}</span>
            <span class="muted">关键词：{{ r.plan.keywords.join('、') || '—' }}</span>
            <span class="muted">近 {{ r.plan.days }} 天 · 命中 {{ r.hits.length }} 条</span>
          </div>
          <ul v-if="r.hits.length" class="hits">
            <li v-for="(h, j) in visibleHits(i, r.hits)" :key="j">
              <code>{{ h.time }}</code> <span class="muted">[{{ h.layer }}]</span> {{ h.text }}
            </li>
          </ul>
          <button v-if="r.hits.length > 8" class="btn small expand" @click="expanded[i] = !expanded[i]">
            {{ expanded[i] ? '收起' : `展开全部 ${r.hits.length} 条` }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.presets {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
}
.preset {
  font-size: 12px;
  padding: 5px 10px;
  border-radius: 999px;
  border: 1px solid rgba(255, 255, 255, 0.1);
  background: rgba(255, 255, 255, 0.04);
  color: #c9cde8;
  cursor: pointer;
  transition: all 0.15s;
}
.preset:hover:not(:disabled) {
  background: rgba(143, 214, 255, 0.12);
  border-color: #8fd6ff55;
}
.preset:disabled {
  opacity: 0.5;
}
.limit-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
}
.limit-input {
  width: 56px;
  padding: 4px 6px;
  border-radius: 8px;
  border: 1px solid rgba(255, 255, 255, 0.12);
  background: rgba(255, 255, 255, 0.05);
  color: #c9cde8;
  font-size: 12px;
}
.citations {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  align-items: center;
  margin-top: 12px;
}
.cite {
  font-size: 12px;
  padding: 4px 10px;
  border-radius: 999px;
  border: 1px solid #8fd6ff33;
  background: rgba(143, 214, 255, 0.08);
  color: #8fd6ff;
  cursor: pointer;
  transition: all 0.15s;
}
.cite:hover {
  background: rgba(143, 214, 255, 0.18);
}
.traj-head {
  cursor: pointer;
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.round {
  margin: 10px 0;
  padding: 10px;
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.07);
}
.round-head {
  display: flex;
  gap: 8px;
  align-items: center;
  flex-wrap: wrap;
}
.hits {
  margin: 8px 0 0;
  padding-left: 18px;
}
.hits li {
  font-size: 12px;
  color: #c9cde8;
  margin: 4px 0;
  line-height: 1.5;
}
.hits code {
  color: #8fd6ff;
  font-size: 11px;
}
.answer {
  font-size: 15px;
  line-height: 1.7;
  margin: 0;
}
.preview-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.preview-close {
  background: transparent;
  border: none;
  color: #8b90b5;
  font-size: 16px;
  cursor: pointer;
}
.preview-close:hover { color: #fff; }
.preview-meta {
  display: flex;
  gap: 8px;
  align-items: center;
  flex-wrap: wrap;
  margin: 8px 0;
  font-size: 12px;
}
.preview-meta code { color: #8fd6ff; }
.preview-imgs {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
  margin: 10px 0;
}
.preview-img {
  width: 220px;
  border-radius: 10px;
  border: 1px solid rgba(255, 255, 255, 0.1);
  object-fit: cover;
}
.preview-text {
  font-size: 13px;
  color: #c9cde8;
  line-height: 1.6;
  margin: 6px 0 0;
  white-space: pre-wrap;
}
.expand {
  margin-top: 8px;
  font-size: 12px;
}
</style>
