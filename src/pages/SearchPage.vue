<script setup lang="ts">
import { ref } from 'vue'
import { search, type FeedPage } from '../api'
import FeedItem from '../components/FeedItem.vue'
import ErrorBox from '../components/ErrorBox.vue'
import LoadingList from '../components/LoadingList.vue'

const keyword = ref('')
const loading = ref(false)
const error = ref<unknown>(null)
const items = ref<Array<Record<string, unknown>>>([])
const searched = ref(false)

function extract(res: FeedPage): Array<Record<string, unknown>> {
  if (Array.isArray(res?.data)) return res.data as Array<Record<string, unknown>>
  return []
}

async function run() {
  const kw = keyword.value.trim()
  if (!kw) return
  loading.value = true
  error.value = null
  searched.value = true
  try {
    const res = await search(kw, 1)
    items.value = extract(res)
  } catch (e) {
    error.value = e
    items.value = []
  } finally {
    loading.value = false
  }
}
</script>

<template>
  <div class="page">
    <div class="searchbar">
      <input
        v-model="keyword"
        class="input"
        type="search"
        placeholder="搜索动态、话题、用户"
        @keyup.enter="run"
      />
      <button class="btn btn-primary" :disabled="loading" @click="run">
        搜索
      </button>
    </div>

    <ErrorBox v-if="error" :error="error" @retry="run" />

    <template v-else>
      <LoadingList v-if="loading" :count="3" />
      <template v-else-if="searched">
        <FeedItem v-for="(item, i) in items" :key="i" :item="item" />
        <div v-if="items.length === 0" class="state">没有找到相关内容</div>
      </template>
      <div v-else class="state">输入关键词后按回车或点击搜索</div>
    </template>
  </div>
</template>

<style scoped>
.searchbar {
  display: flex;
  gap: 8px;
  margin-bottom: 16px;
}

.input {
  flex: 1;
  padding: 9px 14px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--surface);
  color: var(--text);
  font: inherit;
  outline: none;
}

.input:focus {
  border-color: var(--accent);
}
</style>
