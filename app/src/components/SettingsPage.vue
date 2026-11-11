<script setup>
// Settings: which browser opens sites, updates, and where NetLaunch keeps
// its files.
import { computed, reactive, ref, watch } from 'vue';

const props = defineProps({
  // { settings: { browser, browser_path, check_updates }, browsers: [{ id, name, path }], version, can_update }
  // or null while loading.
  view: { type: Object, default: null },
  // async (settings) => saved settings. Throws with a message on failure.
  save: { type: Function, required: true },
  // The update state from App.vue:
  // { status: { current, can_update, available } | null, checking, error, installing, progress }.
  update: { type: Object, required: true },
  vaultPath: { type: String, default: null },
});
const emit = defineEmits(['check-update', 'install-update']);

const form = reactive({ browser: 'edge', browser_path: '', check_updates: true });
watch(
  () => props.view?.settings,
  (s) => {
    if (s) Object.assign(form, { browser: s.browser, browser_path: s.browser_path ?? '', check_updates: s.check_updates });
  },
  { immediate: true },
);

const NAMES = { edge: 'Microsoft Edge', chrome: 'Google Chrome' };
// Every choice, found or not, so the saved one always shows.
const choices = computed(() =>
  ['edge', 'chrome'].map((id) => {
    const found = props.view?.browsers.find((b) => b.id === id);
    return { id, name: NAMES[id], path: found?.path ?? null };
  }),
);

const saving = ref(false);
const saved = ref(false);
const error = ref('');

async function submit() {
  error.value = '';
  saved.value = false;
  saving.value = true;
  try {
    await props.save({
      browser: form.browser,
      browser_path: form.browser === 'custom' ? form.browser_path.trim() || null : null,
      check_updates: form.check_updates,
    });
    saved.value = true;
  } catch (e) {
    error.value = String(e);
  } finally {
    saving.value = false;
  }
}

// The updates checkbox saves by itself: it has no other fields to wait for.
async function toggleUpdates() {
  const s = props.view.settings;
  try {
    await props.save({ ...s, check_updates: form.check_updates });
  } catch (e) {
    error.value = String(e);
  }
}

const percent = computed(() => {
  const p = props.update.progress;
  return p?.total ? Math.min(100, Math.round((p.downloaded / p.total) * 100)) : null;
});
const megabytes = (bytes) => (bytes / 1048576).toFixed(1);
</script>

<template>
  <p v-if="!view" class="muted">Loading the settings…</p>
  <template v-else>
    <form class="card" @submit.prevent="submit">
      <h2>Browser</h2>
      <p class="muted small">The browser NetLaunch opens sites in. Windows that are already open keep theirs.</p>
      <label v-for="c in choices" :key="c.id" class="inline choice">
        <input v-model="form.browser" type="radio" :value="c.id" :disabled="!c.path" />
        <span>
          {{ c.name }}
          <span class="muted small">{{ c.path ?? 'not installed' }}</span>
        </span>
      </label>
      <label class="inline choice">
        <input v-model="form.browser" type="radio" value="custom" />
        <span>Another Chromium-based browser <span class="muted small">(Brave, Vivaldi…)</span></span>
      </label>
      <label v-if="form.browser === 'custom'" class="stack">
        Path of its .exe file
        <input v-model="form.browser_path" required spellcheck="false" placeholder="C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe" />
      </label>
      <p class="muted small">Firefox and Safari can't be used: NetLaunch drives the window through the Chromium debugging connection.</p>
      <p v-if="error" class="banner error" role="alert">{{ error }}</p>
      <div class="form-actions">
        <button type="submit" class="primary" :disabled="saving">{{ saving ? 'Saving…' : 'Save browser' }}</button>
        <span v-if="saved" class="muted small" role="status">Saved. The next Open uses it.</span>
      </div>
    </form>

    <section class="card" aria-labelledby="updates">
      <h2 id="updates">Updates</h2>
      <p>
        NetLaunch <strong>{{ view.version }}</strong>
        <span v-if="update.status && !update.status.available && !update.error" class="tag">up to date</span>
      </p>

      <p v-if="!view.can_update" class="muted small">
        This copy can't update itself: it wasn't built by the Release workflow, so it has no key to check updates against.
        Install a released version from the project's Releases page on GitHub once; after that, updates arrive here.
      </p>
      <template v-else>
        <div v-if="update.status?.available" class="banner ok">
          <strong>Version {{ update.status.available.version }} is available.</strong>
          <p v-if="update.status.available.notes" class="notes">{{ update.status.available.notes }}</p>
          <p v-if="update.installing" role="status">
            {{ percent === null ? 'Downloading…' : percent < 100 ? `Downloading… ${percent}%` : 'Installing… NetLaunch will restart.' }}
            <span v-if="update.progress" class="small">({{ megabytes(update.progress.downloaded) }} MB)</span>
          </p>
          <p v-else>
            <button class="primary" @click="emit('install-update')">Install and restart</button>
            <span class="small"> Windows NetLaunch keeps open are closed first; your sites stay.</span>
          </p>
        </div>
        <p v-if="update.error" class="banner error" role="alert">{{ update.error }}</p>
        <div class="form-actions">
          <button :disabled="update.checking || update.installing" @click="emit('check-update')">
            {{ update.checking ? 'Checking…' : 'Check for updates' }}
          </button>
        </div>
        <label class="inline choice">
          <input v-model="form.check_updates" type="checkbox" @change="toggleUpdates" />
          Check when NetLaunch starts
        </label>
      </template>
    </section>

    <section class="card" aria-labelledby="files">
      <h2 id="files">Files</h2>
      <p v-if="vaultPath" class="small">Vault: <code>{{ vaultPath }}</code></p>
      <p class="small muted">The log (<code>logs\netlaunch.log</code>) and these settings (<code>settings.json</code>) are in the same folder. Updating or uninstalling keeps the vault.</p>
    </section>
  </template>
</template>
