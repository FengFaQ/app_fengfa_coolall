<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { importShuzlmId, sessionInfo, type SessionInfo } from '../api'

const info = ref<SessionInfo | null>(null)
const importText = ref('')
const message = ref('')
const failed = ref(false)

async function refresh() {
  try {
    info.value = await sessionInfo()
  } catch {
    info.value = null
  }
}

async function doImport() {
  message.value = ''
  failed.value = false
  const text = importText.value.trim()
  if (!text) {
    failed.value = true
    message.value = '请先粘贴内容'
    return
  }
  try {
    await importShuzlmId(text)
    message.value = '导入成功，设备标识已更新'
    importText.value = ''
    await refresh()
  } catch (e) {
    failed.value = true
    message.value = e instanceof Error ? e.message : String(e)
  }
}

onMounted(refresh)
</script>

<template>
  <div class="page">
    <h2 class="heading">设置</h2>

    <section class="card">
      <h3>设备标识</h3>
      <p class="muted hint">
        桌面端无法读取数盟 SDK，默认使用随机标识。若遇到验证问题，
        可在官方 App 中点击「复制 OAID」5 次，把完整内容粘贴到下方导入。
      </p>
      <div class="row">
        <span class="label">当前 szlmId</span>
        <code class="value">{{ info?.szlmId ?? '读取中…' }}</code>
      </div>
      <div class="row">
        <span class="label">User-Agent</span>
        <code class="value small">{{ info?.userAgent ?? '读取中…' }}</code>
      </div>

      <textarea
        v-model="importText"
        class="textarea"
        rows="4"
        placeholder="粘贴包含 ShuzlmID: xxxx 的完整文本"
      />
      <div class="actions">
        <button class="btn btn-primary" @click="doImport">导入</button>
        <span v-if="message" :class="failed ? 'err' : 'ok'">{{ message }}</span>
      </div>
    </section>

    <section class="card">
      <h3>关于</h3>
      <p class="muted hint">
        coolall 是非官方客户端，与酷安官方无隶属关系。
        本项目以 AGPL-3.0 许可证开源。
      </p>
      <div class="row">
        <span class="label">版本</span>
        <code class="value">0.1.0</code>
      </div>
    </section>
  </div>
</template>

<style scoped>
.heading {
  font-size: 17px;
  font-weight: 600;
  margin-bottom: 14px;
}

h3 {
  font-size: 14px;
  font-weight: 600;
  margin-bottom: 8px;
}

.hint {
  font-size: 13px;
  margin-bottom: 12px;
}

.row {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-bottom: 10px;
}

.label {
  font-size: 12px;
  color: var(--text-secondary);
}

.value {
  font-family: ui-monospace, 'Cascadia Code', Consolas, monospace;
  font-size: 12px;
  word-break: break-all;
  background: var(--surface-hover);
  padding: 6px 8px;
  border-radius: 6px;
}

.value.small {
  font-size: 11px;
}

.textarea {
  width: 100%;
  padding: 10px 12px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--bg);
  color: var(--text);
  font: inherit;
  font-size: 13px;
  resize: vertical;
  outline: none;
}

.textarea:focus {
  border-color: var(--accent);
}

.actions {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 10px;
}

.ok {
  color: var(--accent);
  font-size: 13px;
}

.err {
  color: var(--danger);
  font-size: 13px;
}
</style>
