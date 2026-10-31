<script setup lang="ts">
import { ref } from 'vue'
import { search, type FeedPage } from '../api'
import FeedItem from '../components/FeedItem.vue'
import ErrorBox from '../components/ErrorBox.vue'
import LoadingList from '../components/LoadingList.vue'

const keyword = ref('')
const loading = ref(false)
const loadingMore = ref(false)
const error = ref<unknown>(null)
const items = ref<Array<Record<string, unknown>>>([])
const searched = ref(false)
const page = ref(1)
const hasMore = ref(false)

let requestSeq = 0

function extract(res: FeedPage): Array<Record<string, unknown>> {
  if (Array.isArray(res?.data)) return res.data as Array<Record<string, unknown>>
  return []
}

/**
 * 执行搜索。
 *
 * reset = true：新关键词，清空并搜第 1 页（回车/点按钮触发）
 * reset = false：加载下一页
 */
async function run(reset = true) {
  const kw = keyword.value.trim()
  if (!kw) return
  if (loadingMore.value) return
  if (!reset && !hasMore.value) return

  const seq = ++requestSeq

  if (reset) {
    page.value = 1
    items.value = []
    hasMore.value = false
    searched.value = true
    loading.value = true
  } else {
    loadingMore.value = true
  }
  error.value = null

  try {
    const res = await search(kw, page.value)
    if (seq !== requestSeq) return
    const next = extract(res)
    items.value = reset ? next : [...items.value, ...next]
    hasMore.value = next.length > 0
    if (next.length > 0) page.value += 1
  } catch (e) {
    if (seq !== requestSeq) return
    error.value = e
  } finally {
    if (seq === requestSeq) {
      loading.value = false
      loadingMore.value = false
    }
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
        @keyup.enter="run(true)"
      />
      <button class="btn btn-primary" :disabled="loading" @click="run(true)">
        搜索
      </button>
    </div>

    <LoadingList v-if="loading" :count="3" />

    <ErrorBox v-else-if="error && items.length === 0" :error="error" @retry="run(true)" />

    <template v-else-if="searched">
      <FeedItem
        v-for="item in items"
        :key="String(item.id ?? item.entityId ?? '')"
        :item="item"
      />

      <div v-if="items.length === 0" class="state">没有找到相关内容</div>

      <template v-else>
        <ErrorBox
          v-if="error && items.length > 0"
          :error="error"
          @retry="run(false)"
        />
        <div v-else-if="hasMore" class="footer">
          <button class="btn" :disabled="loadingMore" @click="run(false)">
            {{ loadingMore ? '加载中…' : '加载更多' }}
          </button>
        </div>
        <div v-else class="state">没有更多了</div>
      </template>
    </template>

    <div v-else class="state">输入关键词后按回车或点击搜索</div>
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

.footer {
  display: flex;
  justify-content: center;
  padding: 14px 0;
}
</style>
