<script setup>
// Add or edit a site, as a small dialog. The five things most sites need
// come first (address, username, password, name, group); the rest sits
// under "More options", whose row says what is currently chosen.
//
// The password field is write-only: it always opens empty, and when
// editing, leaving it empty keeps the stored password. The app never
// sends a stored password to this screen, so the reveal button can only
// show what is being typed now.
import { computed, nextTick, reactive, ref, watch } from 'vue';
import Icon from './Icon.vue';
import Modal from './Modal.vue';

const props = defineProps({
  // null when adding a new site.
  site: { type: Object, default: null },
  // Existing group names, for the Group dropdown.
  groups: { type: Array, default: () => [] },
  // The built-in login recipes: [{ id, name, needs_username }].
  recipes: { type: Array, default: () => [] },
  // async (fields) => void. Throws with the vault's message on failure.
  save: { type: Function, required: true },
});
const emit = defineEmits(['cancel']);

const editing = computed(() => props.site !== null);

const fields = reactive({
  name: props.site?.name ?? '',
  login_url: props.site?.login_url ?? '',
  username: props.site?.username ?? '',
  recipe_id: props.site?.recipe_id ?? 'generic-form',
  allow_self_signed: props.site?.allow_self_signed ?? false,
  profile_mode: props.site?.profile_mode ?? 'fresh',
  window_mode: props.site?.window_mode ?? 'managed',
  notes: props.site?.notes ?? '',
});
// Kept apart from `fields` so it's easy to clear after every save attempt.
const password = ref('');
const showPassword = ref(false);
const moreOpen = ref(false);

/** The URL if it's a full http(s) address, else null. */
function parseUrl(raw) {
  try {
    const url = new URL(raw.trim());
    return url.protocol === 'https:' || url.protocol === 'http:' ? url : null;
  } catch {
    return null;
  }
}

// ----- Name: suggested from the address until the admin types one -----
const nameTouched = ref(editing.value);
watch(
  () => fields.login_url,
  (raw) => {
    if (!nameTouched.value) fields.name = parseUrl(raw)?.host ?? '';
  },
);

// ----- Group: pick one, or "New group…" and type its name -----
// Saved group names are trimmed, so none can start with a space: this
// value can never clash with a real group.
const NEW_GROUP = ' new';
const groupChoice = ref(props.site?.group_name ?? '');
const newGroup = ref('');
/** The group to save: null for none. A new name that matches an existing
 *  group apart from capitals uses the existing one, so "switches" doesn't
 *  become a second group next to "Switches". */
function chosenGroup() {
  if (groupChoice.value !== NEW_GROUP) return groupChoice.value || null;
  const typed = newGroup.value.trim();
  return props.groups.find((g) => g.toLowerCase() === typed.toLowerCase()) ?? (typed || null);
}

// ----- Self-signed: turned on for private https addresses -----
// Devices on the admin's own network almost always have a self-signed
// certificate. When adding an https site on a private address, turn the
// setting on for them, until they change it themselves.
const selfSignedTouched = ref(editing.value);
const suggestSelfSigned = computed(() => !editing.value && isPrivateHttps(fields.login_url));
watch(suggestSelfSigned, (suggest) => {
  if (!selfSignedTouched.value) fields.allow_self_signed = suggest;
});

/** True for https URLs on localhost, *.local/.lan/.internal/.home.arpa, or a private IP. */
function isPrivateHttps(raw) {
  const url = parseUrl(raw);
  if (!url || url.protocol !== 'https:') return false;
  const host = url.hostname.toLowerCase().replace(/^\[|\]$/g, '');
  if (host === 'localhost' || /\.(local|lan|internal|home\.arpa)$/.test(host)) return true;
  if (host === '::1' || /^f[cd]/.test(host) || host.startsWith('fe80:')) return true;
  const ip = host.split('.').map(Number);
  if (ip.length !== 4 || ip.some((n) => !Number.isInteger(n) || n < 0 || n > 255)) return false;
  const [a, b] = ip;
  return a === 10 || a === 127 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) || (a === 169 && b === 254);
}

// Some devices (access points) only ask for a password. Their recipe
// types no username, so the field is hidden and nothing is saved for it.
const recipe = computed(() => props.recipes.find((r) => r.id === fields.recipe_id));
const needsUsername = computed(() => recipe.value?.needs_username !== false);

// What "More options" is set to, shown on its row while it is folded.
const summary = computed(() =>
  [
    recipe.value?.name ?? fields.recipe_id,
    fields.allow_self_signed ? 'self-signed certificate' : null,
    fields.profile_mode === 'persistent' ? 'keeps cookies' : 'fresh profile',
    fields.window_mode === 'released' ? 'window handed over' : null,
  ]
    .filter(Boolean)
    .join(' · '),
);

// What is wrong with each field right now. Nothing is shown until Save
// has been tried once; after that the messages follow the typing.
const problems = computed(() => ({
  login_url: !fields.login_url.trim()
    ? 'Enter the address of the login page'
    : parseUrl(fields.login_url)
      ? ''
      : 'Start the address with https:// or http://',
  username: !needsUsername.value || fields.username.trim() ? '' : 'Enter the username (or pick "Password only" under More options)',
  password: editing.value || password.value ? '' : 'Enter the password',
  name: fields.name.trim() ? '' : 'Give the site a name',
  group:
    groupChoice.value !== NEW_GROUP
      ? ''
      : !newGroup.value.trim()
        ? "Type the new group's name"
        : newGroup.value.trim().toLowerCase() === 'no group'
          ? 'Pick "No group" from the list instead, or type another name'
          : '',
}));
const tried = ref(false);
const shown = (field) => (tried.value ? problems.value[field] : '');

// The vault's answer when Save itself failed.
const error = ref('');
const saving = ref(false);

async function submit() {
  tried.value = true;
  if (Object.values(problems.value).some(Boolean)) {
    // Put the cursor in the first field that needs fixing.
    nextTick(() => document.querySelector('.dialog .invalid')?.focus());
    return;
  }
  error.value = '';
  saving.value = true;
  try {
    await props.save({
      ...fields,
      name: fields.name.trim(),
      login_url: fields.login_url.trim(),
      username: needsUsername.value ? fields.username : '',
      group_name: chosenGroup(),
      notes: fields.notes.trim() || null,
      // Empty when editing means "keep the stored password".
      password: password.value === '' ? null : password.value,
    });
  } catch (e) {
    error.value = String(e);
    // Don't keep the typed password around once it has been sent.
    password.value = '';
    showPassword.value = false;
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <Modal :title="editing ? `Edit ${site.name}` : 'Add site'" wide sticky @cancel="emit('cancel')" @submit="submit">
    <p v-if="error" class="banner error" role="alert">{{ error }}</p>

    <label class="field">
      <span>Login page URL</span>
      <span>
        <input v-model="fields.login_url" :class="{ invalid: shown('login_url') }" autocomplete="off" spellcheck="false" placeholder="https://10.0.0.5:8443/login…" />
        <span v-if="shown('login_url')" class="problem">{{ shown('login_url') }}</span>
        <span v-else-if="editing" class="hint">Logins only fill on <code>{{ site.origin }}</code>.</span>
      </span>
    </label>
    <label v-if="needsUsername" class="field">
      <span>Username</span>
      <span>
        <input v-model="fields.username" :class="{ invalid: shown('username') }" autocomplete="off" spellcheck="false" />
        <span v-if="shown('username')" class="problem">{{ shown('username') }}</span>
      </span>
    </label>
    <div class="field">
      <label for="site-password">Password</label>
      <span>
        <span class="with-button">
          <!-- new-password: stop the webview offering to save or autofill it. -->
          <input
            id="site-password"
            v-model="password"
            :type="showPassword ? 'text' : 'password'"
            :class="{ invalid: shown('password') }"
            autocomplete="new-password"
            spellcheck="false"
            :placeholder="editing ? 'Leave empty to keep the saved password' : ''"
          />
          <button type="button" class="btn outline icon-only" :aria-label="showPassword ? 'Hide what you typed' : 'Show what you typed'" :aria-pressed="showPassword" :disabled="!password" @click="showPassword = !showPassword">
            <Icon :name="showPassword ? 'eyeOff' : 'eye'" />
          </button>
        </span>
        <span v-if="shown('password')" class="problem">{{ shown('password') }}</span>
      </span>
    </div>
    <label class="field">
      <span>Name</span>
      <span>
        <input v-model="fields.name" :class="{ invalid: shown('name') }" autocomplete="off" placeholder="UniFi Controller…" @input="nameTouched = true" />
        <span v-if="shown('name')" class="problem">{{ shown('name') }}</span>
        <span v-else-if="!nameTouched && fields.name" class="hint">Suggested from the address. Change it if you like.</span>
      </span>
    </label>
    <div class="field">
      <label for="site-group">Group</label>
      <span>
        <select id="site-group" v-model="groupChoice">
          <option value="">No group</option>
          <option v-for="g in groups" :key="g" :value="g">{{ g }}</option>
          <option :value="NEW_GROUP">New group…</option>
        </select>
        <input v-if="groupChoice === NEW_GROUP" v-model="newGroup" class="below" :class="{ invalid: shown('group') }" maxlength="40" autocomplete="off" aria-label="New group's name" placeholder="New group's name…" />
        <span v-if="shown('group')" class="problem">{{ shown('group') }}</span>
      </span>
    </div>

    <button type="button" class="more" :aria-expanded="moreOpen" aria-controls="site-more" @click="moreOpen = !moreOpen">
      <Icon :name="moreOpen ? 'down' : 'right'" />
      <strong>More options</strong>
      <span v-if="!moreOpen" class="more-summary">{{ summary }}</span>
    </button>

    <div v-show="moreOpen" id="site-more" class="more-body">
      <label class="field">
        <span>Login page type</span>
        <span>
          <select v-model="fields.recipe_id">
            <option v-for="r in recipes" :key="r.id" :value="r.id">{{ r.name }}</option>
          </select>
          <span class="hint">Two-step: the password is asked on a second page. Password only: no username.</span>
        </span>
      </label>
      <div class="field">
        <label for="site-self-signed">Self-signed certificate</label>
        <span>
          <span class="toggle">
            <input id="site-self-signed" v-model="fields.allow_self_signed" type="checkbox" class="switch" role="switch" @change="selfSignedTouched = true" />
            {{ fields.allow_self_signed ? 'On' : 'Off' }}
          </span>
          <span v-if="suggestSelfSigned && !selfSignedTouched && fields.allow_self_signed" class="hint">Turned on because this is an https address on your own network.</span>
          <span v-else-if="fields.allow_self_signed" class="hint">Certificate errors are ignored for this site's window only.</span>
        </span>
      </div>
      <div class="field" role="radiogroup" aria-label="Browser profile">
        <span>Browser profile</span>
        <span class="radios">
          <label><input v-model="fields.profile_mode" type="radio" value="fresh" />Fresh each time (recommended)</label>
          <label><input v-model="fields.profile_mode" type="radio" value="persistent" />Keep cookies between launches</label>
        </span>
      </div>
      <div class="field" role="radiogroup" aria-label="After login">
        <span>After login</span>
        <span class="radios">
          <label><input v-model="fields.window_mode" type="radio" value="managed" />NetLaunch keeps the window (closes with the app)</label>
          <label><input v-model="fields.window_mode" type="radio" value="released" />Hand the window over to me</label>
        </span>
      </div>
      <label class="field top">
        <span>Notes</span>
        <textarea v-model="fields.notes" rows="2" placeholder="Optional, not encrypted…" />
      </label>
    </div>

    <template #buttons>
      <button type="submit" class="btn accent" :disabled="saving">{{ saving ? 'Saving…' : 'Save' }}</button>
      <button type="button" class="btn outline" :disabled="saving" @click="emit('cancel')">Cancel</button>
    </template>
  </Modal>
</template>
