<script setup>
// Top level: loads the vault status and the site list, keeps the open
// windows up to date, and switches between the list and the add/edit form.
import { onMounted, onUnmounted, reactive, ref } from 'vue';
import * as api from './api.js';
import SiteList from './components/SiteList.vue';
import SiteForm from './components/SiteForm.vue';
import SessionPanel from './components/SessionPanel.vue';

const status = ref(null);
const sites = ref([]);
const loadError = ref('');
// null = the list; { site: null } = adding; { site } = editing.
const form = ref(null);
const notice = ref('');
const recipes = ref([]);
// The result of each site's last Open: { busy } while running, then
// { outcome, message, warnings, fix }. Shown under the site when it has no
// open window (the panel shows those).
const logins = reactive({});
// Open windows by site id: SessionView, kept up to date by events.
const sessions = reactive({});
// Site ids with a Release/Close/Retry running.
const pending = reactive({});
const stopListening = [];

async function reload() {
  loadError.value = '';
  try {
    sites.value = await api.listSites();
  } catch (e) {
    loadError.value = String(e);
  }
}

onMounted(async () => {
  try {
    status.value = await api.vaultStatus();
  } catch (e) {
    status.value = { ok: false, path: null, error: String(e) };
  }
  if (status.value.ok) await reload();
  try {
    recipes.value = await api.listRecipes();
  } catch {
    recipes.value = [{ id: 'generic-form', name: 'Generic login form' }];
  }
  stopListening.push(await api.onSessionChanged((view) => (sessions[view.site_id] = view)));
  stopListening.push(await api.onSessionEnded(({ site_id }) => delete sessions[site_id]));
  // Events only bring changes; start from the current list.
  try {
    for (const view of await api.listSessions()) sessions[view.site_id] = view;
  } catch {
    // The vault error banner already explains it.
  }
});

onUnmounted(() => stopListening.forEach((stop) => stop()));

async function open(site) {
  logins[site.id] = { busy: true };
  try {
    const result = await api.openSite(site.id);
    logins[site.id] = { busy: false, ...result };
  } catch (e) {
    // Refused before a login started (already open, vault error...).
    logins[site.id] = { busy: false, outcome: 'failed', message: String(e), warnings: [] };
  }
  // Updates "last used".
  await reload();
}

// The "Trust this certificate" button: tick the site's self-signed
// setting, then open it again.
async function trustAndOpen(site) {
  try {
    const updated = await api.trustCertificate(site.id);
    await open(updated);
  } catch (e) {
    logins[site.id] = { busy: false, outcome: 'failed', message: String(e), warnings: [] };
  }
}

// The panel's buttons. Errors show under the site in the list.
async function windowAction(view, action) {
  pending[view.site_id] = true;
  try {
    if (action === 'release') await api.releaseWindow(view.site_id);
    else await api.closeWindow(view.site_id);
    if (action === 'retry') {
      const site = sites.value.find((s) => s.id === view.site_id);
      if (site) await open(site);
    }
  } catch (e) {
    logins[view.site_id] = { busy: false, outcome: 'failed', message: String(e), warnings: [] };
  } finally {
    delete pending[view.site_id];
  }
}

function flash(text) {
  notice.value = text;
  setTimeout(() => {
    if (notice.value === text) notice.value = '';
  }, 3000);
}

// SiteForm hands over the fields; we save and return to the list. Errors
// are thrown back so the form can show them next to the fields.
async function save(fields) {
  const editing = form.value.site;
  const saved = editing ? await api.updateSite(editing.id, fields) : await api.addSite(fields);
  form.value = null;
  await reload();
  flash(`${editing ? 'Saved' : 'Added'} ${saved.name}`);
}

async function remove(site) {
  try {
    await api.deleteSite(site.id);
    await reload();
    flash(`Deleted ${site.name}`);
  } catch (e) {
    loadError.value = String(e);
  }
}
</script>

<template>
  <header class="topbar">
    <h1>NetLaunch</h1>
    <button v-if="status?.ok && !form" class="primary" @click="form = { site: null }">Add site</button>
  </header>

  <main>
    <p v-if="!status" class="muted">Opening the vault…</p>

    <div v-else-if="!status.ok" class="banner error" role="alert">
      <strong>The vault couldn't be opened.</strong>
      <p>{{ status.error }}</p>
      <p v-if="status.path" class="muted">Vault file: <code>{{ status.path }}</code></p>
    </div>

    <template v-else>
      <p v-if="notice && !form" class="banner ok" role="status">{{ notice }}</p>
      <p v-if="loadError" class="banner error" role="alert">{{ loadError }}</p>

      <SessionPanel
        v-if="!form && Object.keys(sessions).length"
        :sessions="Object.values(sessions).sort((a, b) => a.site_name.localeCompare(b.site_name))"
        :pending="pending"
        @release="(v) => windowAction(v, 'release')"
        @close="(v) => windowAction(v, 'close')"
        @retry="(v) => windowAction(v, 'retry')"
      />

      <SiteForm
        v-if="form"
        :site="form.site"
        :groups="[...new Set(sites.map((s) => s.group_name).filter(Boolean))]"
        :recipes="recipes"
        :save="save"
        @cancel="form = null"
      />
      <SiteList
        v-else
        :sites="sites"
        :logins="logins"
        :sessions="sessions"
        @add="form = { site: null }"
        @open="open"
        @trust="trustAndOpen"
        @edit="(site) => (form = { site })"
        @delete="remove"
      />
    </template>
  </main>

  <footer v-if="status?.ok && status.path" class="muted footer">Vault: <code>{{ status.path }}</code></footer>
</template>
