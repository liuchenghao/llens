<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'

type Cfg = {
  api_key: string
  base_url: string
  model: string
  recording_enabled: boolean
  diary_lookback_days: number
}

const cfg = ref<Cfg>({
  api_key: '',
  base_url: '',
  model: '',
  recording_enabled: true,
  diary_lookback_days: 1,
})
const saved = ref(false)
const err = ref('')

onMounted(async () => {
  cfg.value = await invoke<Cfg>('get_config')
})

async function save() {
  err.value = ''
  saved.value = false
  try {
    const out = await invoke<Cfg>('save_config', { new: cfg.value })
    cfg.value = out
    saved.value = true
  } catch (e: any) {
    err.value = String(e)
  }
}
</script>

<template>
  <div>
    <h1 class="page">设置</h1>
    <div class="glass card" style="max-width: 560px">
      <label>API Base URL</label>
      <input v-model="cfg.base_url" placeholder="https://apihub.agnes-ai.com/v1" />
      <label>模型</label>
      <input v-model="cfg.model" placeholder="agnes-3.0-flash" />
      <label>API Key（保存在本地 ~/.screenlog/config.json）</label>
      <input v-model="cfg.api_key" type="password" />
      <label style="display:flex; align-items:center; gap:8px">
        <input type="checkbox" v-model="cfg.recording_enabled" style="width:auto" /> 允许后台截屏
      </label>
      <label>日记自动回溯天数（0=仅今天，1=今天+昨天，以此类推）</label>
      <input
        v-model.number="cfg.diary_lookback_days"
        type="number"
        min="0"
        max="30"
        style="max-width: 80px; display: inline-block"
      />
      <div class="muted" style="font-size:11px; margin-top:2px">影响日记页打开时自动补跑的范围</div>

      <div style="margin-top: 16px; display:flex; gap:10px; align-items:center">
        <button class="btn primary" @click="save">保存</button>
        <span v-if="saved" class="muted">已保存</span>
        <span v-if="err" class="chip warn">{{ err }}</span>
      </div>
    </div>
    <div class="muted" style="font-size: 12px; max-width: 560px">
      数据目录：<code>~/.screenlog</code> · 截屏：screenshots/ · 小时日志：logs/ ·
      10分钟汇总：summaries/ · 日记：diary/
    </div>
  </div>
</template>

<style scoped>
label {
  display: block;
  margin: 14px 0 6px;
  font-size: 13px;
  color: #aeb4dc;
}
code {
  color: #8fd6ff;
  font-size: 11px;
}
</style>
