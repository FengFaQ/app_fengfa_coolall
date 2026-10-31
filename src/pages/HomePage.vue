<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { browse, type FeedPage } from '../api'
import FeedItem from '../components/FeedItem.vue'
import ErrorBox from '../components/ErrorBox.vue'
import LoadingList from '../components/LoadingList.vue'

const loading = ref(true)
const loadingMore = ref(false)
const error = ref<unknown>(null)
const items = ref<Array<Record<string, unknown>>>([])
const page = ref(1)
const hasMore = ref(true)

/// 请求序号：用于丢弃过期响应，避免快速切换时旧结果覆盖新内容
let requestSeq = 0

function extract(res: FeedPage): Array<Record<string, unknown>> {
  if (Array.isArray(res?.data)) return res.data as Array<Record<string, unknown>>
  return []
}

/**
 * 加载列表。
 *
 * reset = true 表示首屏/刷新：清空并回到第 1 页。
 * reset = false 表示翻页：追加到已有内容。
 *
 * 注意两个刻意的行为：
 * 1. 翻页失败时**保留已加载内容**，只把错误显示在列表下方，
 *    不会把整页内容替换成错误页（早期版本会，体验很差）。
 * 2. 并发保护：loading / loadingMore 互斥，且用请求序号丢弃过期响应。
 */
async function load(reset = false) {
  if (loadingMore.value) return
  if (!reset && !hasMore.value) return

  const seq = ++requestSeq

  if (reset) {
    page.value = 1
    hasMore.value = true
    loading.value = true
  } else {
    loadingMore.value = true
  }
  error.value = null

  try {
    const res = await browse('home', page.value)
    // 过期响应直接丢弃
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

onMounted(() => load(true))

/**
 * 验证通过并重放成功。
 *
 * 后端在重放时已经拿到了列表数据，直接用它渲染即可，
 * 不要再发一次请求——那会白白消耗重放机会。
 *
 * 注意：这里必须清空 error，否则界面会一直停在验证卡片上。
 */
function onResolved(data: Record<string, unknown>) {
  const next = extract(data as FeedPage)
  items.value = next
  error.value = null
  hasMore.value = next.length > 0
  page.value = next.length > 0 ? 2 : 1
}
</script>

<template>
  <div class="page">
    <div class="toolbar">
      <h2>首页</h2>
      <button class="btn" :disabled="loading" @click="load(true)">
        {{ loading ? '刷新中…' : '刷新' }}
      </button>
    </div>

    <!-- 首屏错误：此时没有内容可保留 -->
    <ErrorBox
      v-if="error && items.length === 0"
      :error="error"
      @retry="load(true)"
      @dismiss="load(true)"
      @resolved="onResolved"
    />

    <template v-else>
      <LoadingList v-if="loading && items.length === 0" />
      <template v-else>
        <FeedItem v-for="item in items" :key="String(item.id ?? item.entityId ?? '')" :item="item" />

        <div v-if="items.length === 0" class="state">暂无内容</div>

        <template v-else>
          <!-- 翻页错误单独展示，不清空已有内容 -->
          <ErrorBox
            v-if="error && items.length > 0"
            :error="error"
            @retry="load(false)"
            @dismiss="load(false)"
            @resolved="onResolved"
          />

          <div v-else-if="hasMore" class="footer">
            <button class="btn" :disabled="loadingMore" @click="load(false)">
              {{ loadingMore ? '加载中…' : '加载更多' }}
            </button>
          </div>

          <div v-else class="state">没有更多了</div>
        </template>
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

.footer {
  display: flex;
  justify-content: center;
  padding: 14px 0;
}
</style>


