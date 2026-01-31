<script setup lang="ts">
interface Props {
  title: string
  icon?: string
  loading?: boolean
}

withDefaults(defineProps<Props>(), {
  icon: '📊',
  loading: false,
})
</script>

<template>
  <div class="card card--glow">
    <div class="card__header">
      <span class="card__title">{{ title }}</span>
      <span class="card__icon">{{ icon }}</span>
    </div>
    <div class="card__body">
      <div v-if="loading" class="skeleton-container">
        <div class="skeleton" style="height: 48px; width: 120px; margin-bottom: 8px;" />
        <div class="skeleton" style="height: 16px; width: 80px;" />
      </div>
      <slot v-else />
    </div>
  </div>
</template>

<style lang="scss" scoped>
.skeleton-container {
  padding: $spacing-2 0;
}
</style>
