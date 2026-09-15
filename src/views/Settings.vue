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
const dataRoot = ref('')
const newRoot = ref('')
const rootBusy = ref(false)
const rootSaved = ref(false)
const rootErr = ref('')

onMounted(async () => {
  cfg.value = await invoke<Cfg>('get_config').catch(() => cfg.value)
  dataRoot.value = await invoke<string>('data_root').catch(() => '')
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

async function changeDataRoot() {
  if (!newRoot.value.trim()) return
  rootBusy.value = true
  rootErr.value = ''
  rootSaved.value = false
  try {
    const out = await invoke<string>('set_data_root', { newPath: newRoot.value.trim() })
    dataRoot.value = out
    newRoot.value = ''
    rootSaved.value = true
    cfg.value = await invoke<Cfg>('get_config').catch(() => cfg.value)
  } catch (e: any) {
    rootErr.value = String(e)
  } finally {
    rootBusy.value = false
  }
}

async function resetDataRoot() {
  rootBusy.value = true
  rootErr.value = ''
  rootSaved.value = false
  try {
    const out = await invoke<string>('reset_data_root')
    dataRoot.value = out
    rootSaved.value = true
    cfg.value = await invoke<Cfg>('get_config').catch(() => cfg.value)
  } catch (e: any) {
    rootErr.value = String(e)
  } finally {
    rootBusy.value = false
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
      <label>API Key（保存在本地 config.json）</label>
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
        <button class="btn primary" @click="save">保存 LLM 设置</button>
        <span v-if="saved" class="muted">已保存</span>
        <span v-if="err" class="chip warn">{{ err }}</span>
      </div>
    </div>

    <div class="glass card" style="max-width: 560px; margin-top:14px">
      <h3>数据目录</h3>
      <div class="muted" style="font-size:12px; margin-bottom:10px">
        当前：<code>{{ dataRoot || '…' }}</code>
      </div>
      <label>更改数据目录</label>
      <div style="display:flex; gap:8px; align-items:center">
        <input v-model="newRoot" placeholder="例如 ~/mydata 或 /path/to/dir" style="flex:1" />
        <button class="btn" :disabled="rootBusy || !newRoot.trim()" @click="changeDataRoot">
          {{ rootBusy ? '…' : '应用' }}
        </button>
        <button class="btn" :disabled="rootBusy" @click="resetDataRoot">恢复默认</button>
      </div>
      <div v-if="rootSaved" class="muted" style="font-size:12px; margin-top:6px">已切换数据目录，页面已刷新</div>
      <div v-if="rootErr" class="chip warn" style="margin-top:6px">{{ rootErr }}</div>
      <div class="muted" style="font-size:11px; margin-top:10px; line-height:1.6">
        目录结构：screenshots/ · logs/ · summaries/ · diary/<br />
        切换后会自动在新目录创建子目录并迁移 LLM 配置读取位置。
      </div>
    </div>

    <div class="muted" style="font-size: 12px; max-width: 560px; margin-top:12px">
      数据目录可通过上方更改，默认 <code>~/.screenlog</code>
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
h3 {
  margin: 0 0 8px;
  font-size: 14px;
  font-weight: 600;
  color: #aeb4dc;
  text-transform: uppercase;
  letter-spacing: 0.08em;
}
button.btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
</style>
