<script setup>
// Add or edit a site. The password field is write-only: it always opens
// empty, and when editing, leaving it empty keeps the stored password.
// The app never sends a stored password to this screen.
import { computed, reactive, ref } from 'vue';

const props = defineProps({
  // null when adding a new site.
  site: { type: Object, default: null },
  // Existing group names, offered as suggestions.
  groups: { type: Array, default: () => [] },
  // async (fields) => void. Throws with the vault's message on failure.
  save: { type: Function, required: true },
});
const emit = defineEmits(['cancel']);

const editing = computed(() => props.site !== null);

const fields = reactive({
  name: props.site?.name ?? '',
  group_name: props.site?.group_name ?? '',
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

const error = ref('');
const saving = ref(false);

async function submit() {
  error.value = '';
  saving.value = true;
  try {
    await props.save({
      ...fields,
      group_name: fields.group_name.trim() || null,
      notes: fields.notes.trim() || null,
      // Empty when editing means "keep the stored password".
      password: password.value === '' ? null : password.value,
    });
  } catch (e) {
    error.value = String(e);
  } finally {
    // Don't keep the typed password around once it has been sent.
    password.value = '';
    saving.value = false;
  }
}
</script>

<template>
  <form class="site-form" @submit.prevent="submit">
    <h2>{{ editing ? `Edit ${site.name}` : 'Add a site' }}</h2>
    <p v-if="error" class="banner error" role="alert">{{ error }}</p>

    <label>
      Name
      <input v-model="fields.name" required placeholder="UniFi Controller" />
    </label>

    <label>
      <span>Group <span class="muted small">(optional)</span></span>
      <input v-model="fields.group_name" list="group-names" placeholder="Wireless" />
      <datalist id="group-names">
        <option v-for="g in groups" :key="g" :value="g" />
      </datalist>
    </label>

    <label>
      Login page URL
      <input v-model="fields.login_url" required type="url" placeholder="https://10.0.0.5:8443/login" />
      <span v-if="editing" class="muted small">Logins only fill on <code>{{ site.origin }}</code> (scheme, host and port from this URL).</span>
    </label>

    <label>
      Username
      <input v-model="fields.username" required autocomplete="off" spellcheck="false" />
    </label>

    <label>
      Password
      <!-- new-password: stop the webview offering to save or autofill it. -->
      <input
        v-model="password"
        type="password"
        :required="!editing"
        autocomplete="new-password"
        spellcheck="false"
        :placeholder="editing ? 'Leave empty to keep the stored password' : ''"
      />
      <span class="muted small">Encrypted on this PC. NetLaunch never shows it again.</span>
    </label>

    <label>
      Login recipe
      <select v-model="fields.recipe_id">
        <option value="generic-form">Generic login form</option>
      </select>
    </label>

    <fieldset>
      <legend>Certificate</legend>
      <label class="inline">
        <input v-model="fields.allow_self_signed" type="checkbox" />
        This site uses a self-signed certificate
      </label>
      <span v-if="fields.allow_self_signed" class="muted small">Certificate errors are ignored for this site's window only. Turn it on only for devices on your own network.</span>
    </fieldset>

    <fieldset>
      <legend>Browser profile</legend>
      <label class="inline"><input v-model="fields.profile_mode" type="radio" value="fresh" /> Fresh each time (recommended)</label>
      <label class="inline"><input v-model="fields.profile_mode" type="radio" value="persistent" /> Keep cookies between launches</label>
    </fieldset>

    <fieldset>
      <legend>After login</legend>
      <label class="inline"><input v-model="fields.window_mode" type="radio" value="managed" /> NetLaunch keeps the window (closes with the app)</label>
      <label class="inline"><input v-model="fields.window_mode" type="radio" value="released" /> Hand the window over to me</label>
    </fieldset>

    <label>
      <span>Notes <span class="muted small">(optional, not encrypted)</span></span>
      <textarea v-model="fields.notes" rows="3" />
    </label>

    <div class="form-actions">
      <button type="submit" class="primary" :disabled="saving">{{ saving ? 'Saving…' : editing ? 'Save changes' : 'Add site' }}</button>
      <button type="button" :disabled="saving" @click="emit('cancel')">Cancel</button>
    </div>
  </form>
</template>
