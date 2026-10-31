<script setup lang="ts">
import { computed } from 'vue'
import { useRouter } from 'vue-router'

const props = defineProps<{
  item: Record<string, unknown>
}>()

const router = useRouter()

function pick(...keys: string[]): string {
  for (const k of keys) {
    const v = props.item[k]
    if (typeof v === 'string' && v.trim()) return v
    if (typeof v === 'number') return String(v)
  }
  return ''
}

const id = computed(() => pick('id', 'entityId', 'feedId'))
const username = computed(() => pick('username', 'userName', 'author'))
const title = computed(() => pick('title', 'messageTitle'))
const message = computed(() => pick('message', 'content', 'description'))
const avatime = computed(() => pick('dateline', 'createTime', 'time'))

// 文本型动态直接展示 message，其余展示 title
const summary = computed(() => {
  const m = message.value
  if (m) return m.length > 220 ? `${m.slice(0, 220)}…` : m
  return title.value
})

function open() {
  if (id.value) router.push(`/feed/${encodeURIComponent(id.value)}`)
}
</script>

<template>
  <article class="card feed-item" @click="open">
    <header class="meta">
      <span v-if="username" class="user">{{ username }}</span>
      <span v-if="avatime" class="muted time">{{ avatime }}</span>
    </header>
    <p v-if="summary" class="body">{{ summary }}</p>
    <p v-else class="muted body">（无文本内容）</p>
  </article>
</template>

<style scoped>
.feed-item {
  cursor: pointer;
}

.meta {
  display: flex;
  align-items: baseline;
  gap: 10px;
  margin-bottom: 6px;
}

.user {
  font-weight: 600;
  font-size: 13px;
  color: var(--accent);
}

.time {
  font-size: 12px;
}

.body {
  font-size: 14px;
  white-space: pre-wrap;
  word-break: break-word;
}
</style>
