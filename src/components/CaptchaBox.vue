<script setup lang="ts">
import { computed, ref } from 'vue'
import { ApiError } from '../api'
import { extractValidate, submitCaptcha } from '../api/captcha'

const props = defineProps<{
  error: unknown
}>()

const emit = defineEmits<{ retry: []; dismiss: [] }>()

const validateText = ref('')
const submitting = ref(false)
const message = ref('')
const failed = ref(false)

/// 从后端错误里取出挑战信息
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

async function submit() {
  const c = challenge.value
  if (!c) return
  message.value = ''
  failed.value = false

  const validate = extractValidate(validateText.value)
  if (!validate) {
    failed.value = true
    message.value = '请先粘贴验证结果'
    return
  }

  submitting.value = true
  try {
    await submitCaptcha({
      challengeId: c.id,
      field: c.field,
      validate,
    })
    message.value = '验证已提交，正在重试…'
    validateText.value = ''
    emit('retry')
  } catch (e) {
    failed.value = true
    message.value = e instanceof Error ? e.message : String(e)
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <div class="captcha-box">
    <!-- 需要验证码时，给出明确的操作入口，而不是只说"需要验证" -->
    <template v-if="challenge">
      <div class="icon">验证</div>
      <p class="title">需要完成人机验证</p>
      <p class="hint">
        酷安对未登录访问启用了安全验证。请点击下方按钮完成验证，
        然后把结果粘贴回来。
      </p>

      <div class="actions">
        <a
          class="btn"
          href="https://account.coolapk.com/auth/login?type=mobile"
          target="_blank"
          rel="noopener noreferrer"
        >
          打开验证页面
        </a>
      </div>

      <textarea
        v-model="validateText"
        class="textarea"
        rows="3"
        placeholder="粘贴验证结果（或包含 validate=... 的整段内容）"
      />

      <div class="actions">
        <button class="btn btn-primary" :disabled="submitting" @click="submit">
          {{ submitting ? '提交中…' : '提交并重试' }}
        </button>
        <button class="btn" @click="emit('dismiss')">稍后再说</button>
      </div>

      <p v-if="message" :class="failed ? 'err' : 'ok'">{{ message }}</p>
    </template>

    <template v-else>
      <div class="icon">!</div>
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
  max-width: 520px;
  margin: 0 auto;
}

.icon {
  width: auto;
  min-width: 40px;
  height: 40px;
  padding: 0 12px;
  margin: 0 auto 12px;
  border-radius: 20px;
  background: var(--accent);
  color: #fff;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-weight: 700;
  font-size: 14px;
}

.title {
  font-weight: 600;
  margin-bottom: 8px;
}

.hint {
  color: var(--text-secondary);
  font-size: 13px;
  margin-bottom: 16px;
  line-height: 1.7;
}

.textarea {
  width: 100%;
  padding: 10px 12px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--surface);
  color: var(--text);
  font: inherit;
  font-size: 13px;
  resize: vertical;
  outline: none;
  margin-bottom: 12px;
}

.textarea:focus {
  border-color: var(--accent);
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
}

.err {
  color: var(--danger);
  font-size: 13px;
}
</style>
