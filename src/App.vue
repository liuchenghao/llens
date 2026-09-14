<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import Overview from './views/Overview.vue'
import Diary from './views/Diary.vue'
import Board from './views/Board.vue'
import Qa from './views/Qa.vue'
import Settings from './views/Settings.vue'
import { ref, onMounted } from 'vue'
import './style.css'

type Tab = 'overview' | 'diary' | 'board' | 'qa' | 'settings'

const tabs: { id: Tab; label: string; icon: string }[] = [
  { id: 'overview', label: '概览', icon: '◉' },
  { id: 'diary', label: '日记', icon: '✎' },
  { id: 'board', label: '看板', icon: '▤' },
  { id: 'qa', label: '问答', icon: '✳' },
  { id: 'settings', label: '设置', icon: '⚙' },
]

const active = ref<Tab>('overview')

async function getStatus() {
  try {
    return await invoke<any>('get_status')
  } catch {
    return { recording: false, data_root: '(unavailable)' }
  }
}

const status = ref<any>({ recording: false, data_root: '' })

onMounted(async () => {
  status.value = await getStatus()
  setInterval(async () => {
    status.value = await getStatus()
  }, 5000)
})

async function setRecording(on: boolean) {
  await invoke('set_recording', { on })
  status.value = await getStatus()
}
</script>

<template>
  <div class="app">
    <aside class="nav glass">
      <div class="logo">
        <span class="logo-dot" :class="{ rec: status?.recording }" />
        LLENS
      </div>
      <nav>
        <button
          v-for="t in tabs"
          :key="t.id"
          :class="['tab', { active: active === t.id }]"
          @click="active = t.id"
        >
          <span class="tab-icon">{{ t.icon }}</span>
          <span>{{ t.label }}</span>
        </button>
      </nav>
      <div class="status">
        <button
          :class="['rec-btn', { on: status?.recording }]"
          @click="setRecording(!status?.recording)"
        >
          {{ status?.recording ? '⏸ 录制中' : '▶ 已停止' }}
        </button>
        <div class="data-root" :title="status?.data_root">
          数据目录 · {{ status?.data_root || '—' }}
        </div>
      </div>
    </aside>

    <main class="content">
      <Overview v-if="active === 'overview'" />
      <Diary v-else-if="active === 'diary'" />
      <Board v-else-if="active === 'board'" />
      <Qa v-else-if="active === 'qa'" />
      <Settings v-else />
    </main>
  </div>
</template>
