<script setup lang="ts">
defineProps<{
  count?: number
}>()
</script>

<template>
  <div>
    <div v-for="i in count ?? 4" :key="i" class="skeleton" />
  </div>
</template>
