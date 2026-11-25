<script setup>
// The right pane: everything about the selected site. Its name and
// address, the buttons that act on it, what its last login or its open
// window is saying, and its saved settings.
//
// The password is never here: the app doesn't send it to the window.
import { computed } from 'vue';
import Icon from './Icon.vue';
import { isBusy, lastUsed } from '../states.js';

const props = defineProps({
  site: { type: Object, required: true },
  // From siteStatus(): { kind, label, message, warnings, fix } or null.
  status: { type: Object, default: null },
  // The site's open window (SessionView), or null.
  session: { type: Object, default: null },
  // True while Open is starting, or a Release/Close/Retry is running.
  opening: { type: Boolean, default: false },
  pending: { type: Boolean, default: false },
  // The built-in login recipes: [{ id, name }], for the recipe's name.
  recipes: { type: Array, default: () => [] },
});
const emit = defineEmits(['open', 'edit', 'copy', 'delete', 'trust', 'release', 'close', 'retry']);

const recipeName = computed(() => props.recipes.find((r) => r.id === props.site.recipe_id)?.name ?? props.site.recipe_id);
// The window's buttons wait while the login itself is still running.
const windowBusy = computed(() => !props.session || isBusy(props.session.state) || props.pending);
</script>

<template>
  <article class="details" :aria-label="site.name">
    <p class="eyebrow">{{ site.group_name || 'No group' }}</p>
    <h1>{{ site.name }}</h1>
    <p class="url">{{ site.login_url }}</p>

    <div class="actions">
      <button class="btn accent wide" :disabled="opening || !!session" :title="session ? 'This site\'s window is already open' : 'Open and log in (Enter)'" @click="emit('open')">
        <Icon name="open" />{{ opening ? 'Opening…' : 'Open' }}
      </button>
      <button class="btn outline" title="Edit this site (F2)" @click="emit('edit')"><Icon name="edit" />Edit</button>
      <button class="btn outline" @click="emit('copy')"><Icon name="copy" />Copy URL</button>
      <button class="btn outline" title="Delete this site (Del)" @click="emit('delete')"><Icon name="delete" />Delete</button>
    </div>

    <!-- The last login's result, or the open window's live state. -->
    <section v-if="status" class="statusbox" :class="status.kind" aria-label="Login status">
      <span class="dot"></span>
      <div class="statusbox-text">
        <strong>{{ status.label }}</strong>
        <p v-if="status.message">{{ status.message }}</p>
        <p v-for="w in status.warnings" :key="w">Warning: {{ w }}</p>
      </div>
      <div v-if="session || status.fix" class="statusbox-actions">
        <button v-if="status.fix === 'trust_certificate'" class="btn outline" title="Only for devices on your own network" @click="emit('trust')">
          Trust this certificate and open again
        </button>
        <template v-if="session">
          <button v-if="session.window_mode === 'managed'" class="btn outline" :disabled="windowBusy" title="Disconnect NetLaunch and leave the window open" @click="emit('release')">
            Release
          </button>
          <button class="btn outline" :disabled="windowBusy" @click="emit('close')">Close window</button>
          <button class="btn outline" :disabled="windowBusy" title="Close the window and log in again" @click="emit('retry')">Retry</button>
        </template>
      </div>
    </section>

    <h2>Login</h2>
    <dl class="facts">
      <div><dt>Username</dt><dd>{{ site.username || 'None: this device only asks for a password' }}</dd></div>
      <div><dt>Password</dt><dd>Saved, encrypted on this PC</dd></div>
      <div><dt>Login page type</dt><dd>{{ recipeName }}</dd></div>
      <div><dt>Certificate</dt><dd>{{ site.allow_self_signed ? 'Self-signed: errors are ignored for this site\'s window only' : 'Checked as usual' }}</dd></div>
    </dl>

    <h2>Window</h2>
    <dl class="facts">
      <div><dt>Browser profile</dt><dd>{{ site.profile_mode === 'persistent' ? 'Keeps cookies between launches' : 'Fresh each time' }}</dd></div>
      <div><dt>After login</dt><dd>{{ site.window_mode === 'released' ? 'The window is handed over to you' : 'NetLaunch keeps the window (closes with the app)' }}</dd></div>
      <div><dt>Last used</dt><dd class="num">{{ lastUsed(site) }}</dd></div>
    </dl>

    <template v-if="site.notes">
      <h2>Notes</h2>
      <p class="notes">{{ site.notes }}</p>
    </template>
  </article>
</template>
