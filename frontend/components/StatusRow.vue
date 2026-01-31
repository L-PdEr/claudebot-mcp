<script setup lang="ts">
interface Props {
  label: string
  status: 'online' | 'degraded' | 'offline' | 'unknown'
  detail?: string
}

const props = withDefaults(defineProps<Props>(), {
  detail: '',
})

const statusClass = computed(() => {
  return `status-badge__dot--${props.status}`
})

const statusText = computed(() => {
  switch (props.status) {
    case 'online':
      return 'Online'
    case 'degraded':
      return 'Degraded'
    case 'offline':
      return 'Offline'
    default:
      return 'Unknown'
  }
})
</script>

<template>
  <div class="status-row">
    <span class="status-row__label">{{ label }}</span>
    <div class="status-row__status">
      <span v-if="detail" class="status-row__detail">{{ detail }}</span>
      <div class="status-badge">
        <span class="status-badge__dot" :class="statusClass" />
        <span class="status-badge__text">{{ statusText }}</span>
      </div>
    </div>
  </div>
</template>

<style lang="scss" scoped>
.status-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: $spacing-3 0;
  border-bottom: 1px solid var(--border);

  &:last-child {
    border-bottom: none;
  }

  &__label {
    color: var(--text-secondary);
  }

  &__status {
    display: flex;
    align-items: center;
    gap: $spacing-3;
  }

  &__detail {
    font-family: var(--font-mono);
    font-size: $font-size-sm;
    color: var(--text-muted);
  }
}
</style>
