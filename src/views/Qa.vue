<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

type Plan = {
  keywords: string[]
  layer: string
  days: number
  need_more: boolean
}
type Hit = { layer: string; time: string; text: string; source: string }

const question = ref('')
const rounds = ref<{ plan: Plan; hits: Hit[] }[]>([])
const answer = ref('')
const busy = ref(false)
const err = ref('')

// Agentic flow, hard limits: <= 3 search rounds, each round <= 20 hits x 200 chars.
async function ask() {
  if (!question.value.trim() || busy.value) return
  busy.value = true
  err.value = ''
  answer.value = ''
  rounds.value = []

  try {
    // round 1: model plans the search
    let plan = await invoke<Plan>('qa_plan', { question: question.value })
    let hits = await invoke<Hit[]>('qa_search', { plan })
    rounds.value.push({ plan, hits })

    // round 2: model may refine once (only if it still wants more AND we have budget)
    if (plan.need_more) {
      const plan2 = await invoke<Plan>('qa_refine', { question: question.value, prevHits: hits })
      if (plan2.keywords.length && plan2.days > 0) {
        const hits2 = await invoke<Hit[]>('qa_search', { plan: plan2 })
        rounds.value.push({ plan: plan2, hits: hits2 })
        hits = [...hits, ...hits2]
      }
    }

    // final: answer from accumulated context
    answer.value = await invoke<string>('qa_answer', { question: question.value, hits })
  } catch (e: any) {
    err.value = String(e)
  } finally {
    busy.value = false
  }
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
      <div style="display: flex; gap: 10px; margin-top: 10px; align-items: center">
        <button class="btn primary" @click="ask" :disabled="busy || !question.trim()">
          {{ busy ? '检索中…' : '提问' }}
        </button>
        <span class="muted" style="font-size: 11px">agentic 检索 · 最多 3 轮 · 单轮 ≤ 20 条 × 200 字</span>
      </div>
    </div>

    <div v-if="err" class="glass card"><div class="chip warn">出错：{{ err }}</div></div>

    <div v-if="rounds.length" class="glass card">
      <h3>检索轨迹</h3>
      <div v-for="(r, i) in rounds" :key="i" class="round">
        <div class="round-head">
          <span class="chip">第 {{ i + 1 }} 轮</span>
          <span class="chip warn">{{ r.plan.layer }}</span>
          <span class="muted">关键词：{{ r.plan.keywords.join('、') || '—' }}</span>
          <span class="muted">近 {{ r.plan.days }} 天 · 命中 {{ r.hits.length }} 条</span>
        </div>
        <ul v-if="r.hits.length" class="hits">
          <li v-for="(h, j) in r.hits.slice(0, 8)" :key="j">
            <code>{{ h.time }}</code> <span class="muted">[{{ h.layer }}]</span> {{ h.text }}
          </li>
        </ul>
      </div>
    </div>

    <div v-if="answer" class="glass card">
      <h3>回答</h3>
      <p class="answer">{{ answer }}</p>
    </div>
  </div>
</template>

<style scoped>
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
</style>
