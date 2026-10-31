<script setup lang="ts">
import { RouterView, useRoute, useRouter } from 'vue-router'

const route = useRoute()
const router = useRouter()

const tabs = [
  { key: 'home', label: '首页', path: '/home' },
  { key: 'search', label: '搜索', path: '/search' },
  { key: 'settings', label: '设置', path: '/settings' },
]

function isActive(key: string) {
  return route.path.startsWith(`/${key}`)
}
</script>

<template>
  <div class="shell">
    <header class="topbar">
      <div class="brand">coolall</div>
      <nav class="tabs">
        <button
          v-for="tab in tabs"
          :key="tab.key"
          class="tab"
          :class="{ active: isActive(tab.key) }"
          @click="router.push(tab.path)"
        >
          {{ tab.label }}
        </button>
      </nav>
    </header>

    <main class="content">
      <RouterView />
    </main>
  </div>
</template>

<style scoped>
.shell {
  display: flex;
  flex-direction: column;
  height: 100%;
}

.topbar {
  display: flex;
  align-items: center;
  gap: 24px;
  padding: 12px 20px;
  background: var(--surface);
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}

.brand {
  font-weight: 700;
  font-size: 16px;
  color: var(--accent);
  letter-spacing: 0.3px;
}

.tabs {
  display: flex;
  gap: 4px;
}

.tab {
  padding: 5px 14px;
  border-radius: 8px;
  color: var(--text-secondary);
  font-size: 13px;
  transition: all 0.15s ease;
}

.tab:hover {
  background: var(--surface-hover);
  color: var(--text);
}

.tab.active {
  background: var(--accent-soft);
  color: var(--accent);
  font-weight: 600;
}

.content {
  flex: 1;
  overflow-y: auto;
}
</style>
