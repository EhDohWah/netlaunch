<script setup>
// "Open windows": every Edge window NetLaunch is looking after, with its
// live status and Release / Close / Retry. Spec: the session panel (M6).
import { STATE_LABELS, isBusy, stateKind } from '../states.js';

defineProps({
  // SessionView[] from the Rust side.
  sessions: { type: Array, required: true },
  // Site ids with a Release/Close/Retry in progress.
  pending: { type: Object, required: true },
});
const emit = defineEmits(['release', 'close', 'retry']);
</script>

<template>
  <section class="panel" aria-labelledby="open-windows">
    <h2 id="open-windows">Open windows</h2>
    <ul class="sites">
      <li v-for="s in sessions" :key="s.site_id" class="site">
        <div class="site-main">
          <div class="site-name">
            {{ s.site_name }}
            <span class="tag" :title="s.window_mode === 'managed' ? 'Closes when NetLaunch closes' : 'Handed over once the login is finished'">
              {{ s.window_mode }}
            </span>
          </div>
          <div class="login-status" :class="stateKind(s.state)" role="status">
            <strong>{{ STATE_LABELS[s.state] ?? s.state }}</strong>
            <!-- "Logged in." would only repeat the label. -->
            <span v-if="s.message && s.message !== 'Logged in.'"> · {{ s.message }}</span>
            <div v-for="w in s.warnings" :key="w" class="small">Warning: {{ w }}</div>
          </div>
        </div>
        <div class="site-actions">
          <button
            v-if="s.window_mode === 'managed'"
            :disabled="isBusy(s.state) || pending[s.site_id]"
            title="Disconnect NetLaunch and leave the window open"
            @click="emit('release', s)"
          >
            Release
          </button>
          <button :disabled="isBusy(s.state) || pending[s.site_id]" @click="emit('close', s)">Close</button>
          <button :disabled="isBusy(s.state) || pending[s.site_id]" title="Close the window and log in again" @click="emit('retry', s)">
            Retry
          </button>
        </div>
      </li>
    </ul>
  </section>
</template>
