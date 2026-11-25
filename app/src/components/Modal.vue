<script setup>
// A small dialog over the app: a title, content, and buttons bottom-right.
// Esc cancels; Enter in a field submits (the content is a form). While it
// is open the keyboard stays inside it, and closing it puts the focus back
// where it was.
import { onMounted, onUnmounted, ref } from 'vue';

const props = defineProps({
  title: { type: String, required: true },
  // Room for a two-column form.
  wide: { type: Boolean, default: false },
  // A click outside doesn't close it: there is typed text to lose.
  sticky: { type: Boolean, default: false },
});
const emit = defineEmits(['cancel', 'submit']);
const box = ref(null);
// What had the focus before the dialog opened, to give it back after.
let before = null;

// Tab stays inside the dialog: from the last control it goes to the
// first, and back. Otherwise it would reach the app behind the dialog.
function key(event) {
  if (event.key === 'Escape') return emit('cancel');
  if (event.key !== 'Tab') return;
  const stops = [...box.value.querySelectorAll('input, select, textarea, button')].filter((el) => !el.disabled && el.offsetParent !== null);
  if (!stops.length) return;
  const first = stops[0];
  const last = stops[stops.length - 1];
  const inside = box.value.contains(document.activeElement);
  if (event.shiftKey && (!inside || document.activeElement === first)) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && (!inside || document.activeElement === last)) {
    event.preventDefault();
    first.focus();
  }
}
onMounted(() => {
  before = document.activeElement;
  window.addEventListener('keydown', key);
  // Start in the first field, or on the first button.
  (box.value.querySelector('input, select, textarea') ?? box.value.querySelector('button'))?.focus();
});
onUnmounted(() => {
  window.removeEventListener('keydown', key);
  before?.focus?.();
});
</script>

<template>
  <div class="backdrop" @mousedown.self="props.sticky || emit('cancel')">
    <form ref="box" class="dialog" :class="{ wide }" novalidate role="dialog" aria-modal="true" :aria-label="title" @submit.prevent="emit('submit')">
      <div class="dialog-body">
        <h2>{{ title }}</h2>
        <slot />
      </div>
      <div class="dialog-foot"><slot name="buttons" /></div>
    </form>
  </div>
</template>
