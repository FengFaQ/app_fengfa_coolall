<script setup lang="ts">
import { ApiError } from '../api'

const props = defineProps<{
  error: unknown
}>()

const emit = defineEmits<{ retry: [] }>()

function describe(err: unknown) {
  if (err instanceof ApiError) {
    return { message: err.message, retryable: err.retryable }
  }
  return { message: '发生未知错误', retryable: true }
}
</script>

<template>
  <div class="error-box">
    <div class="icon">!</div>
    <p class="message">{{ describe(props.error).message }}</p>
    <button
      v-if="describe(props.error).retryable"
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
