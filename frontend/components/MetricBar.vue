<script setup lang="ts">
interface Props {
  label: string
  value: string | number
  percent: number
  variant?: 'default' | 'success' | 'warning' | 'error'
}

const props = withDefaults(defineProps<Props>(), {
  variant: 'default',
})

const fillClass = computed(() => {
  if (props.variant !== 'default') {
    return `progress__fill--${props.variant}`
  }
  // Auto-determine based on percent
  if (props.percent >= 90) return 'progress__fill--error'
  if (props.percent >= 75) return 'progress__fill--warning'
  return ''
})
</script>

<template>
  <div class="metric-bar">
    <div class="progress__header">
      <span class="metric-bar__label">{{ label }}</span>
      <span class="metric-bar__value">{{ value }}</span>
    </div>
    <div class="progress__track">
      <div
        class="progress__fill"
        :class="fillClass"
        :style="{ width: `${Math.min(percent, 100)}%` }"
      />
    </div>
  </div>
</template>

<style lang="scss" scoped>
.metric-bar {
  &__label {
    color: var(--text-secondary);
  }

  &__value {
    font-weight: $font-weight-semibold;
    font-family: var(--font-mono);
  }

  & + & {
    margin-top: $spacing-4;
  }
}
</style>
