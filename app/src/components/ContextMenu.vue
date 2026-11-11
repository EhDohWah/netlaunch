<script setup>
// A right-click menu. `items` is a list of
// { label, hint, icon, disabled, action } or { separator: true } or
// { label, icon, children: [...] } (a submenu that opens in place).
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import Icon from './Icon.vue';

const props = defineProps({
  x: { type: Number, required: true },
  y: { type: Number, required: true },
  items: { type: Array, required: true },
});
const emit = defineEmits(['close']);

const root = ref(null);
const opened = ref(null);
// Moved back inside the window when it would hang over the edge.
const shift = ref({ x: 0, y: 0 });
const style = computed(() => ({ left: `${props.x - shift.value.x}px`, top: `${props.y - shift.value.y}px` }));

function fit() {
  const box = root.value.getBoundingClientRect();
  shift.value = {
    x: Math.max(0, props.x + box.width + 8 - window.innerWidth),
    y: Math.max(0, props.y + box.height + 8 - window.innerHeight),
  };
}

function run(item) {
  if (item.disabled) return;
  if (item.children) {
    opened.value = opened.value === item.label ? null : item.label;
    nextTick(fit);
    return;
  }
  emit('close');
  item.action();
}

function outside(event) {
  if (!root.value?.contains(event.target)) emit('close');
}
function key(event) {
  if (event.key === 'Escape') emit('close');
}

onMounted(() => {
  fit();
  root.value.querySelector('button:not(:disabled)')?.focus();
  // `true`: see the click before anything else handles it.
  window.addEventListener('mousedown', outside, true);
  window.addEventListener('keydown', key);
  window.addEventListener('resize', outside);
});
onUnmounted(() => {
  window.removeEventListener('mousedown', outside, true);
  window.removeEventListener('keydown', key);
  window.removeEventListener('resize', outside);
});
</script>

<template>
  <div ref="root" class="menu" role="menu" :style="style">
    <template v-for="(item, i) in items" :key="i">
      <div v-if="item.separator" class="menu-sep"></div>
      <template v-else>
        <button class="menu-item" role="menuitem" :disabled="item.disabled" @click="run(item)">
          <Icon v-if="item.icon" :name="item.icon" />
          <span v-else class="icon"></span>
          <span class="grow">{{ item.label }}</span>
          <span v-if="item.hint" class="hint">{{ item.hint }}</span>
          <Icon v-if="item.children" :name="opened === item.label ? 'down' : 'right'" />
        </button>
        <template v-if="item.children && opened === item.label">
          <button v-for="child in item.children" :key="child.label" class="menu-item sub" role="menuitem" :disabled="child.disabled" @click="run(child)">
            <span class="icon"></span>
            <span class="grow">{{ child.label }}</span>
          </button>
        </template>
      </template>
    </template>
  </div>
</template>
