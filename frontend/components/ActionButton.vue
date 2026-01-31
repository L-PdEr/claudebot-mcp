<script setup lang="ts">
interface Props {
  icon?: string
  variant?: 'primary' | 'secondary' | 'ghost' | 'danger'
  size?: 'sm' | 'md' | 'lg'
  loading?: boolean
  disabled?: boolean
}

withDefaults(defineProps<Props>(), {
  icon: '',
  variant: 'primary',
  size: 'md',
  loading: false,
  disabled: false,
})

defineEmits<{
  (e: 'click'): void
}>()
</script>

<template>
  <button
    class="btn"
    :class="[
      `btn--${variant}`,
      `btn--${size}`,
      { 'btn--loading': loading },
    ]"
    :disabled="disabled || loading"
    @click="$emit('click')"
  >
    <span v-if="loading" class="btn__spinner" />
    <span v-else-if="icon" class="btn__icon">{{ icon }}</span>
    <span class="btn__text">
      <slot />
    </span>
  </button>
</template>

<style lang="scss" scoped>
.btn {
  &__icon {
    margin-right: $spacing-2;
  }

  &__spinner {
    width: 16px;
    height: 16px;
    border: 2px solid transparent;
    border-top-color: currentColor;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    margin-right: $spacing-2;
  }

  &--loading {
    pointer-events: none;
    opacity: 0.8;
  }
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
