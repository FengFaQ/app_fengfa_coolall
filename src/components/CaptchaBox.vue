<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { ApiError, setPublicFallback } from '../api'
import {
  closeCaptchaWindow,
  completeCaptcha,
  openCaptchaWindow,
} from '../api/captcha'

const props = defineProps<{
  error: unknown
}>()

const emit = defineEmits<{
  retry: []
  dismiss: []
  /** 验证成功并重放完成，携带重放得到的响应体 */
  resolved: [data: Record<string, unknown>]
}>()

const windowOpen = ref(false)
const submitting = ref(false)
const message = ref('')
const failed = ref(false)

const challenge = computed(() => {
  if (!(props.error instanceof ApiError)) return null
  const d = props.error.detail
  if (d.kind !== 'captcha_needed') return null
  return d.challenge
})

function describe(err: unknown) {
  if (err instanceof ApiError) {
    return { message: err.message, retryable: err.retryable }
  }
  return { message: '发生未知错误', retryable: true }
}

/** 打开应用内嵌验证窗口。 */
async function openWindow() {
  message.value = ''
  failed.value = false
  try {
    await openCaptchaWindow()
    windowOpen.value = true
    message.value = '请在弹出的窗口中完成验证，完成后点下方按钮'
  } catch (e) {
    failed.value = true
    message.value = e instanceof Error ? e.message : String(e)
  }
}

/** 用户点「我已完成验证」：后端使用验证窗口回传的 validate 重放原请求。 */
async function confirmDone() {
  submitting.value = true
  message.value = ''
  failed.value = false
  try {
    const data = await completeCaptcha()
    message.value = '验证通过'
    windowOpen.value = false
    emit('resolved', data)
  } catch (e) {
    failed.value = true
    message.value = e instanceof Error ? e.message : String(e)
  } finally {
    submitting.value = false
  }
}

/** 用户放弃验证，改为只看公开页。 */
async function giveUp() {
  try {
    await closeCaptchaWindow()
  } catch {
    // 窗口可能已关闭，忽略
  }
  windowOpen.value = false
  try {
    await setPublicFallback(true)
  } catch {
    // 开启失败也继续重试，最坏情况是再次看到本对话框
  }
  emit('dismiss')
}

onBeforeUnmount(() => {
  // 组件卸载时关掉残留窗口，避免孤儿窗口
  if (windowOpen.value) {
    void closeCaptchaWindow().catch(() => {})
  }
})
</script>

<template>
  <div class="captcha-box">
    <template v-if="challenge">
      <div class="badge">验证</div>
      <p class="title">需要完成人机验证</p>
      <p class="hint">
        酷安对未登录访问启用了安全验证。点击下方按钮，在弹出的窗口中完成验证，
        完成后回到这里点「我已完成验证」，内容会自动加载。
      </p>

      <div class="actions">
        <button class="btn btn-primary" @click="openWindow">
          {{ windowOpen ? '重新打开验证窗口' : '打开验证窗口' }}
        </button>
        <button class="btn btn-primary" :disabled="submitting" @click="confirmDone">
          {{ submitting ? '正在读取凭证…' : '我已完成验证' }}
        </button>
      </div>

      <div class="actions">
        <button class="btn" @click="giveUp">放弃验证，只看公开页</button>
      </div>

      <p v-if="message" :class="failed ? 'err' : 'ok'">{{ message }}</p>
      <button v-if="failed" class="btn" @click="emit('retry')">
        重新加载详情
      </button>
    </template>

    <template v-else>
      <div class="badge danger">!</div>
      <p class="title">{{ describe(props.error).message }}</p>
      <div class="actions">
        <button
          v-if="describe(props.error).retryable"
          class="btn btn-primary"
          @click="emit('retry')"
        >
          重试
        </button>
      </div>
    </template>
  </div>
</template>

<style scoped>
.captcha-box {
  padding: 36px 20px;
  text-align: center;
  max-width: 540px;
  margin: 0 auto;
}

.badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 40px;
  height: 40px;
  padding: 0 12px;
  margin: 0 auto 12px;
  border-radius: 20px;
  background: var(--accent);
  color: #fff;
  font-weight: 700;
  font-size: 14px;
}

.badge.danger {
  background: var(--danger);
  border-radius: 50%;
  min-width: 40px;
  padding: 0;
  font-size: 20px;
}

.title {
  font-weight: 600;
  margin-bottom: 8px;
}

.hint {
  color: var(--text-secondary);
  font-size: 13px;
  margin-bottom: 18px;
  line-height: 1.7;
}

.actions {
  display: flex;
  gap: 10px;
  justify-content: center;
  align-items: center;
  margin-bottom: 12px;
  flex-wrap: wrap;
}

.ok {
  color: var(--accent);
  font-size: 13px;
  line-height: 1.6;
}

.err {
  color: var(--danger);
  font-size: 13px;
  line-height: 1.6;
}
</style>
