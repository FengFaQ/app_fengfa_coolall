<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { detail, detailWithCaptcha, replies, type FeedPage } from '../api'
import { sanitizeHtml } from '../utils/sanitize'
import ErrorBox from '../components/ErrorBox.vue'
import LoadingList from '../components/LoadingList.vue'

const route = useRoute()
const router = useRouter()

const loading = ref(true)
const error = ref<unknown>(null)
const content = ref<Record<string, unknown> | null>(null)
const replyList = ref<Array<Record<string, unknown>>>([])
const replyError = ref<unknown>(null)

function str(v: unknown): string {
  return typeof v === 'string' ? v : typeof v === 'number' ? String(v) : ''
}

/// 公开页 HTML 来自外部站点，必须清洗后再渲染。
/// 渲染环境持有 Tauri invoke，未清洗的 XSS 可越权调用后端命令。
const safePublicHtml = computed(() =>
  sanitizeHtml(str(content.value?.html)),
)

/**
 * 加载详情。
 *
 * `verifyCaptcha` 对齐参考项目 FeedViewModel.refresh() 的两段式：
 * - 首次打开传 false：失败直接报错，不打扰用户
 * - 用户点「重试」传 true：允许触发人机验证
 *
 * 这样服务端下发的挑战才会变成验证窗口，而不是一直被当成普通错误。
 */
async function load(verifyCaptcha = false) {
  const id = String(route.params.id ?? '')
  loading.value = true
  error.value = null
  replyError.value = null
  try {
    content.value = verifyCaptcha
      ? await detailWithCaptcha(id)
      : await detail(id)

    // 公开页兜底时没有评论，跳过
    if (!content.value?.isPublic) {
      try {
        const r = await replies(id, 1)
        replyList.value = Array.isArray((r as FeedPage).data)
          ? ((r as FeedPage).data as Array<Record<string, unknown>>)
          : []
      } catch (e) {
        replyError.value = e
      }
    }
  } catch (e) {
    error.value = e
  } finally {
    loading.value = false
  }
}

/** 用户点重试：这次允许触发人机验证。 */
function retryWithCaptcha() {
  void load(true)
}

onMounted(() => load(false))

/**
 * 验证通过并重放成功。
 *
 * 后端在重放时已经拿到了内容，直接用它渲染即可，
 * 不要再发一次 detail 请求——那会白白多消耗一次请求，
 * 而且若凭证是一次性的会立刻失效。
 */
async function onResolved(data: Record<string, unknown>) {
  content.value = data
  error.value = null
  replyError.value = null
  replyList.value = []
  if (!data.isPublic) {
    try {
      const r = await replies(String(route.params.id ?? ''), 1)
      replyList.value = Array.isArray((r as FeedPage).data)
        ? ((r as FeedPage).data as Array<Record<string, unknown>>)
        : []
    } catch (e) {
      replyError.value = e
    }
  }
}
</script>

<template>
  <div class="page">
    <button class="btn back" @click="router.back()">← 返回</button>

    <LoadingList v-if="loading" :count="2" />

    <ErrorBox v-else-if="error" :error="error" @retry="retryWithCaptcha" @resolved="onResolved" />

    <template v-else-if="content">
      <!-- 公开页兜底：后端只返回 HTML -->
      <template v-if="content.isPublic">
        <div class="notice">
          该内容来自公开分享页，可能不完整。
        </div>
        <div class="public-html" v-html="safePublicHtml" />
      </template>

      <template v-else>
        <article class="card">
          <header class="meta">
            <span class="user">{{ str(content.username) || '匿名' }}</span>
            <span class="muted">{{ str(content.dateline) }}</span>
          </header>
          <h1 v-if="str(content.title)" class="title">
            {{ str(content.title) }}
          </h1>
          <p class="body">{{ str(content.message) }}</p>
        </article>

        <h3 class="section-title">评论</h3>
        <ErrorBox v-if="replyError" :error="replyError" @retry="retryWithCaptcha" />
        <template v-else>
          <div v-for="(r, i) in replyList" :key="i" class="card reply">
            <header class="meta">
              <span class="user">{{ str(r.username) || '匿名' }}</span>
              <span class="muted">{{ str(r.dateline) }}</span>
            </header>
            <p class="body">{{ str(r.message) }}</p>
          </div>
          <div v-if="replyList.length === 0" class="state">还没有评论</div>
        </template>
      </template>
    </template>
  </div>
</template>

<style scoped>
.back {
  margin-bottom: 14px;
}

.meta {
  display: flex;
  gap: 10px;
  align-items: baseline;
  margin-bottom: 8px;
  font-size: 13px;
}

.user {
  font-weight: 600;
  color: var(--accent);
}

.title {
  font-size: 16px;
  font-weight: 600;
  margin-bottom: 8px;
}

.body {
  white-space: pre-wrap;
  word-break: break-word;
}

.section-title {
  font-size: 14px;
  font-weight: 600;
  margin: 20px 0 10px;
}

.notice {
  padding: 10px 14px;
  border-radius: 8px;
  background: var(--accent-soft);
  color: var(--accent);
  font-size: 13px;
  margin-bottom: 12px;
}

.public-html {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 16px;
  overflow-x: auto;
}
</style>


