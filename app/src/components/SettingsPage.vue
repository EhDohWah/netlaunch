<script setup>
// Settings, laid out like the Add/Edit dialog: sections with the label on
// the left and the control on the right. Browser, Appearance, Updates and
// where NetLaunch keeps its files.
import { computed, reactive, ref, watch } from 'vue';
import { theme, toggleTheme } from '../theme.js';

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

const form = reactive({ browser: 'edge', browser_path: '', check_updates: true, mica: true });
watch(
  () => props.view?.settings,
  (s) => {
    if (s) Object.assign(form, { browser: s.browser, browser_path: s.browser_path ?? '', check_updates: s.check_updates, mica: s.mica });
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
      ...props.view.settings,
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

// The switches save by themselves: they have no other fields to wait for.
async function saveSwitch(name) {
  try {
    await props.save({ ...props.view.settings, [name]: form[name] });
  } catch (e) {
    error.value = String(e);
  }
}

function setTheme(wanted) {
  if (theme.value !== wanted) toggleTheme();
}

const percent = computed(() => {
  const p = props.update.progress;
  return p?.total ? Math.min(100, Math.round((p.downloaded / p.total) * 100)) : null;
});
const megabytes = (bytes) => (bytes / 1048576).toFixed(1);
</script>

<template>
  <p v-if="!view" class="muted">Loading the settings…</p>
  <div v-else class="settings">
    <h3>Browser</h3>
    <form class="field" @submit.prevent="submit">
      <span>Open sites in</span>
      <span class="radios">
        <label v-for="c in choices" :key="c.id">
          <input v-model="form.browser" type="radio" :value="c.id" :disabled="!c.path" />
          {{ c.name }}
          <span class="path">{{ c.path ?? 'not installed' }}</span>
        </label>
        <label>
          <input v-model="form.browser" type="radio" value="custom" />
          Another Chromium-based browser
          <span class="path">Brave, Vivaldi…</span>
        </label>
        <input
          v-if="form.browser === 'custom'"
          v-model="form.browser_path"
          class="indent"
          required
          spellcheck="false"
          aria-label="Path of the browser's .exe file"
          placeholder="C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe"
        />
        <span v-if="error" class="problem" role="alert">{{ error }}</span>
        <span class="row">
          <button type="submit" class="btn outline" :disabled="saving">{{ saving ? 'Saving…' : 'Apply' }}</button>
          <span v-if="saved" class="hint" role="status">Saved. The next Open uses it.</span>
        </span>
        <span class="hint">Windows that are already open keep their browser. Firefox and Safari can't be used: NetLaunch drives the window through the Chromium debugging connection.</span>
      </span>
    </form>

    <h3>Appearance</h3>
    <div class="field" role="radiogroup" aria-label="Theme">
      <span>Theme</span>
      <span class="radios">
        <label><input type="radio" name="theme" :checked="theme === 'light'" @change="setTheme('light')" />Light</label>
        <label><input type="radio" name="theme" :checked="theme === 'dark'" @change="setTheme('dark')" />Dark</label>
      </span>
    </div>
    <div class="field">
      <label for="set-mica">Mica background</label>
      <span>
        <span class="toggle">
          <input id="set-mica" v-model="form.mica" type="checkbox" class="switch" role="switch" :disabled="!view.mica_supported" @change="saveSwitch('mica')" />
          {{ form.mica && view.mica_supported ? 'On' : 'Off' }}
        </span>
        <span class="hint">{{ view.mica_supported ? 'The desktop wallpaper tints the window behind the panes, as in Windows 11 apps.' : 'Needs Windows 11.' }}</span>
      </span>
    </div>

    <h3 id="updates">Updates</h3>
    <div class="field">
      <span>This version</span>
      <span class="toggle">
        NetLaunch {{ view.version }}
        <span v-if="update.status && !update.status.available && !update.error" class="badge ok">up to date</span>
      </span>
    </div>
    <div v-if="!view.can_update" class="field">
      <span></span>
      <span class="hint">
        This copy can't update itself: it wasn't built by the Release workflow, so it has no key to check updates against.
        Install a released version from the project's Releases page on GitHub once; after that, updates arrive here.
      </span>
    </div>
    <template v-else>
      <div v-if="update.status?.available" class="field">
        <span>Available</span>
        <span class="update-box">
          <strong>Version {{ update.status.available.version }}</strong>
          <span v-if="update.status.available.notes" class="notes">{{ update.status.available.notes }}</span>
          <span v-if="update.installing" role="status">
            {{ percent === null ? 'Downloading…' : percent < 100 ? `Downloading… ${percent}%` : 'Installing… NetLaunch will restart.' }}
            <span v-if="update.progress" class="small">({{ megabytes(update.progress.downloaded) }} MB)</span>
            <progress v-if="percent !== null" :value="percent" max="100"></progress>
          </span>
          <span v-else class="row">
            <button class="btn accent" @click="emit('install-update')">Install and restart</button>
            <span class="hint">Windows NetLaunch keeps open are closed first; your sites stay.</span>
          </span>
        </span>
      </div>
      <div class="field">
        <span>Check now</span>
        <span>
          <button class="btn outline" :disabled="update.checking || update.installing" @click="emit('check-update')">
            {{ update.checking ? 'Checking…' : 'Check for updates' }}
          </button>
          <span v-if="update.error" class="problem" role="alert">{{ update.error }}</span>
        </span>
      </div>
      <div class="field">
        <label for="set-updates">Check when NetLaunch starts</label>
        <span class="toggle">
          <input id="set-updates" v-model="form.check_updates" type="checkbox" class="switch" role="switch" @change="saveSwitch('check_updates')" />
          {{ form.check_updates ? 'On' : 'Off' }}
        </span>
      </div>
    </template>

    <h3>Files</h3>
    <div v-if="vaultPath" class="field">
      <span>Vault</span>
      <span><code>{{ vaultPath }}</code></span>
    </div>
    <div class="field">
      <span>Log and settings</span>
      <span class="hint">In the same folder: <code>logs\netlaunch.log</code> and <code>settings.json</code>. Updating or uninstalling keeps the vault.</span>
    </div>
  </div>
</template>
