<script setup lang="ts">
import { computed } from 'vue'
import { ApiError } from '../api'
import CaptchaBox from './CaptchaBox.vue'

const props = defineProps<{
  error: unknown
}>()

const emit = defineEmits<{
  retry: []
  resolved: [data: Record<string, unknown>]
}>()

/// 需要人机验证时，交给专用组件处理（它要展示操作入口与输入框）
const needsCaptcha = computed(
  () => props.error instanceof ApiError && props.error.needsCaptcha,
)

function describe(err: unknown) {
  if (err instanceof ApiError) {
    return { message: err.message, retryable: err.retryable }
  }
  return { message: '发生未知错误', retryable: true }
}
</script>

<template>
  <CaptchaBox
    v-if="needsCaptcha"
    :error="error"
    @retry="emit('retry')"
    @dismiss="emit('retry')"
    @resolved="(d) => emit('resolved', d)"
  />

  <div v-else class="error-box">
    <div class="icon">!</div>
    <p class="message">{{ describe(error).message }}</p>
    <button
      v-if="describe(error).retryable"
      class="btn btn-primary"
      @click="emit('retry')"
    >
      重试
    </button>
  </div>
</template>

<style scoped>
.error-box {
  padding: 44px 20px;
  text-align: center;
}

.icon {
  width: 40px;
  height: 40px;
  margin: 0 auto 12px;
  border-radius: 50%;
  background: var(--danger);
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  font-weight: 700;
  font-size: 20px;
}

.message {
  color: var(--text-secondary);
  margin-bottom: 16px;
}
</style>
