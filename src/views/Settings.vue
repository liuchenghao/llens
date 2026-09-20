<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'

type Cfg = {
  api_key: string
  base_url: string
  model: string
  recording_enabled: boolean
  diary_lookback_days: number
  retention_image_days: number
  retention_json_days: number
}

const cfg = ref<Cfg>({
  api_key: '',
  base_url: '',
  model: '',
  recording_enabled: true,
  diary_lookback_days: 1,
  retention_image_days: 90,
  retention_json_days: 365,
})
const saved = ref(false)
const err = ref('')
const dataRoot = ref('')
const newRoot = ref('')
const rootBusy = ref(false)
const rootSaved = ref(false)
const rootErr = ref('')

// PRD §6 权限引导（P2）：检测屏幕录制权限 + 打开系统设置。
const perm = ref<{ granted: boolean; detail: string } | null>(null)
async function checkPerm() {
  try {
    perm.value = await invoke<{ granted: boolean; detail: string }>('screen_permission')
  } catch (e: any) {
    perm.value = { granted: false, detail: `检测失败：${String(e).slice(0, 60)}` }
  }
}
async function openPermSettings() {
  await invoke('open_screen_recording_settings').catch(() => {})
}

onMounted(async () => {
  cfg.value = await invoke<Cfg>('get_config').catch(() => cfg.value)
  dataRoot.value = await invoke<string>('data_root').catch(() => '')
  checkPerm()
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

// PRD §4.6 数据清理：按保留天数立即清理一次，显示效果。
const cleanBusy = ref(false)
const cleanMsg = ref('')
async function runCleanupNow() {
  cleanBusy.value = true
  cleanMsg.value = ''
  try {
    const rep = await invoke<{ removed_images: number; removed_json: number; freed_bytes: number }>(
      'run_cleanup',
    )
    const mb = (rep.freed_bytes / 1024 / 1024).toFixed(2)
    cleanMsg.value = `已清理 ${rep.removed_images} 张图片、${rep.removed_json} 个 JSON，释放 ${mb} MB`
  } catch (e: any) {
    cleanMsg.value = `清理失败：${String(e).slice(0, 80)}`
  } finally {
    cleanBusy.value = false
  }
}
</script>

<template>
  <div>
    <h1 class="page">设置</h1>
    <div class="grid">
      <!-- 左列 -->
      <div class="glass card">
        <h3>LLM 设置</h3>
        <label>API Base URL</label>
        <input v-model="cfg.base_url" placeholder="https://apihub.agnes-ai.com/v1" />
        <label>模型</label>
        <input v-model="cfg.model" placeholder="agnes-3.0-flash" />
        <label>API Key（保存在本地 config.json）</label>
        <input v-model="cfg.api_key" type="password" />
        <div style="margin-top: 14px; display:flex; gap:10px; align-items:center">
          <button class="btn primary" @click="save">保存</button>
          <span v-if="saved" class="muted">已保存</span>
          <span v-if="err" class="chip warn">{{ err }}</span>
        </div>
      </div>

      <div class="glass card">
        <h3>屏幕录制权限（PRD §6 / P2）</h3>
        <div style="display:flex; align-items:center; gap:12px">
          <div
            class="perm-dot"
            :style="perm ? (perm.granted ? { background: '#4ade80', boxShadow: '0 0 10px rgba(74,222,128,.6)' } : { background: '#f87171', boxShadow: '0 0 10px rgba(248,113,113,.6)' }) : { background: '#5a5f8a' }"
          />
          <div style="flex:1">
            <div style="font-size:13px; font-weight:600; color:#fff">
              {{ perm ? (perm.granted ? '已授权 — 可正常记录' : '未授权 — 仅记录休息时段') : '检测中…' }}
            </div>
            <div class="muted" style="font-size:11px; margin-top:2px">{{ perm ? perm.detail : ' ' }}</div>
          </div>
        </div>
        <div style="margin-top:12px; display:flex; gap:10px; align-items:center">
          <button class="btn" @click="checkPerm">重新检测</button>
          <button class="btn" @click="openPermSettings">打开系统权限设置</button>
        </div>

        <div class="divider" />

        <label style="display:flex; align-items:center; gap:8px">
          <input type="checkbox" v-model="cfg.recording_enabled" style="width:auto" /> 允许后台截屏
        </label>
        <label>日记自动回溯天数（0=仅今天，1=今天+昨天）</label>
        <input
          v-model.number="cfg.diary_lookback_days"
          type="number"
          min="0"
          max="30"
          style="max-width: 80px; display: inline-block"
        />
        <div class="muted" style="font-size:11px; margin-top:4px; line-height:1.5">
          未授权时 screencapture 无法产出图像，LLens 将时段标记为「休息」不生成截图；授权后自动恢复。
        </div>
        <div style="margin-top:12px; display:flex; gap:10px; align-items:center">
          <button class="btn primary" @click="save">保存</button>
          <span v-if="saved" class="muted">已保存</span>
        </div>
      </div>

      <div class="glass card">
        <h3>数据保留（PRD §4.6）</h3>
        <label>图片保留天数（0=永久保留）</label>
        <input
          v-model.number="cfg.retention_image_days"
          type="number"
          min="0"
          max="3650"
          style="max-width: 80px; display: inline-block"
        />
        <label>JSON 日志保留天数（0=永久保留）</label>
        <input
          v-model.number="cfg.retention_json_days"
          type="number"
          min="0"
          max="3650"
          style="max-width: 80px; display: inline-block"
        />
        <div class="muted" style="font-size:11px; margin-top:6px; line-height:1.5">
          采集循环每 6 小时自动清理一次超期数据；也可手动立即清理。
        </div>
        <div style="margin-top:10px; display:flex; gap:10px; align-items:center">
          <button class="btn" :disabled="cleanBusy" @click="runCleanupNow">
            {{ cleanBusy ? '清理中…' : '立即清理' }}
          </button>
          <span v-if="cleanMsg" class="muted" style="font-size:12px">{{ cleanMsg }}</span>
        </div>
        <div style="margin-top:12px; display:flex; gap:10px; align-items:center">
          <button class="btn primary" @click="save">保存</button>
          <span v-if="saved" class="muted">已保存</span>
        </div>
      </div>

      <div class="glass card">
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
        <div v-if="rootSaved" class="muted" style="font-size:12px; margin-top:6px">已切换数据目录</div>
        <div v-if="rootErr" class="chip warn" style="margin-top:6px">{{ rootErr }}</div>
        <div class="muted" style="font-size:11px; margin-top:10px; line-height:1.6">
          目录结构：screenshots/ · logs/ · summaries/ · diary/<br />
          默认 <code>~/.screenlog</code>，切换后自动迁移。
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
label {
  display: block;
  margin: 10px 0 4px;
  font-size: 13px;
  color: #aeb4dc;
}

/* 左右两列布局，卡片等高 + 超出滚动 */
.grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 14px;
  align-items: stretch;
}
@media (max-width: 900px) {
  .grid {
    grid-template-columns: 1fr;
  }
}
.grid .glass.card {
  min-height: 0;
  max-height: calc(100vh - 120px);
  overflow-y: auto;
}

.divider {
  border: none;
  border-top: 1px solid rgba(255,255,255,0.08);
  margin: 14px 0;
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
.perm-dot {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  flex-shrink: 0;
  transition: all 0.3s;
}
</style>
