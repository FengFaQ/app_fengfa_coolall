<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { browse, type FeedPage } from '../api'
import FeedItem from '../components/FeedItem.vue'
import ErrorBox from '../components/ErrorBox.vue'
import LoadingList from '../components/LoadingList.vue'

const loading = ref(true)
const error = ref<unknown>(null)
const items = ref<Array<Record<string, unknown>>>([])
const page = ref(1)
const hasMore = ref(true)

function extract(res: FeedPage): Array<Record<string, unknown>> {
  if (Array.isArray(res?.data)) return res.data as Array<Record<string, unknown>>
  return []
}

async function load(reset = false) {
  if (reset) {
    page.value = 1
    hasMore.value = true
    items.value = []
  }
  loading.value = true
  error.value = null
  try {
    const res = await browse('home', page.value)
    const next = extract(res)
    items.value = reset ? next : [...items.value, ...next]
    hasMore.value = next.length > 0
    if (next.length > 0) page.value += 1
  } catch (e) {
    error.value = e
  } finally {
    loading.value = false
  }
}

onMounted(() => load(true))
</script>

<template>
  <div class="page">
    <div class="toolbar">
      <h2>首页</h2>
      <button class="btn" :disabled="loading" @click="load(true)">刷新</button>
    </div>

    <ErrorBox v-if="error" :error="error" @retry="load(true)" />

    <template v-else>
      <LoadingList v-if="loading && items.length === 0" />
      <template v-else>
        <FeedItem v-for="(item, i) in items" :key="i" :item="item" />
        <div v-if="items.length === 0" class="state">暂无内容</div>
        <div v-else-if="!hasMore" class="state">没有更多了</div>
      </template>
    </template>
  </div>
</template>

<style scoped>
.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 14px;
}

h2 {
  font-size: 17px;
  font-weight: 600;
}
</style>
