<script setup>
// A small dialog over the app: a title, content, and buttons bottom-right.
// Esc cancels; Enter in a field submits (the content is a form).
import { onMounted, onUnmounted, ref } from 'vue';

defineProps({ title: { type: String, required: true } });
const emit = defineEmits(['cancel', 'submit']);
const box = ref(null);

function key(event) {
  if (event.key === 'Escape') emit('cancel');
}
onMounted(() => {
  window.addEventListener('keydown', key);
  // Start in the first field, or on the first button.
  (box.value.querySelector('input, select, textarea') ?? box.value.querySelector('button'))?.focus();
});
onUnmounted(() => window.removeEventListener('keydown', key));
</script>

<template>
  <div class="backdrop" @mousedown.self="emit('cancel')">
    <form ref="box" class="dialog" role="dialog" aria-modal="true" :aria-label="title" @submit.prevent="emit('submit')">
      <div class="dialog-body">
        <h2>{{ title }}</h2>
        <slot />
      </div>
      <div class="dialog-foot"><slot name="buttons" /></div>
    </form>
  </div>
</template>
