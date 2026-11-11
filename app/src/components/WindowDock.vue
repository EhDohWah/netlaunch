<script setup>
// "Open windows": every browser window NetLaunch is looking after, with
// its live status and Release / Close / Retry. Docked under the site list;
// it folds to its title row.
import { ref } from 'vue';
import Icon from './Icon.vue';
import { STATE_LABELS, isBusy, stateKind } from '../states.js';

defineProps({
  // SessionView[] from the Rust side.
  sessions: { type: Array, required: true },
  // Site ids with a Release/Close/Retry in progress.
  pending: { type: Object, required: true },
});
const emit = defineEmits(['release', 'close', 'retry', 'select']);
const folded = ref(false);
</script>

<template>
  <section class="dock" aria-label="Open windows">
    <button class="fold dock-title" :aria-expanded="!folded" @click="folded = !folded">
      <Icon :name="folded ? 'right' : 'down'" />
      Open windows
      <span class="count">{{ sessions.length }}</span>
    </button>
    <ul v-if="!folded">
      <li v-for="s in sessions" :key="s.site_id" @click="emit('select', s.site_id)">
        <span class="name">{{ s.site_name }}</span>
        <span class="dock-status" role="status">
          <span class="badge" :class="stateKind(s.state)">{{ STATE_LABELS[s.state] ?? s.state }}</span>
          <span class="badge" :title="s.window_mode === 'managed' ? 'Closes when NetLaunch closes' : 'Handed over once the login is finished'">{{ s.window_mode }}</span>
          <!-- "Logged in." would only repeat the badge. -->
          <span v-if="s.message && s.message !== 'Logged in.'" class="dock-text" :title="s.message">{{ s.message }}</span>
          <span v-for="w in s.warnings" :key="w" class="dock-text warn-text" :title="w">Warning: {{ w }}</span>
        </span>
        <span class="dock-actions">
          <button
            v-if="s.window_mode === 'managed'"
            class="btn small"
            :disabled="isBusy(s.state) || pending[s.site_id]"
            title="Disconnect NetLaunch and leave the window open"
            @click.stop="emit('release', s)"
          >
            Release
          </button>
          <button class="btn small" :disabled="isBusy(s.state) || pending[s.site_id]" @click.stop="emit('close', s)">Close</button>
          <button class="btn small" :disabled="isBusy(s.state) || pending[s.site_id]" title="Close the window and log in again" @click.stop="emit('retry', s)">Retry</button>
        </span>
      </li>
    </ul>
  </section>
</template>
