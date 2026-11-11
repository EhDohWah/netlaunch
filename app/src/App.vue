<script setup>
// Top level: the sidebar and its three pages (Dashboard, Group List,
// Settings). Loads the vault status and the site list, keeps the open
// windows up to date, shows the add/edit form over a page, and looks for
// updates.
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue';
import * as api from './api.js';
import SiteList from './components/SiteList.vue';
import SiteForm from './components/SiteForm.vue';
import SessionPanel from './components/SessionPanel.vue';
import SettingsPage from './components/SettingsPage.vue';
import { theme, toggleTheme } from './theme.js';

const PAGES = [
  { id: 'dashboard', label: 'Dashboard' },
  { id: 'groups', label: 'Group List' },
  { id: 'settings', label: 'Settings' },
];
// Settings sits at the bottom of the sidebar, apart from the other pages.
const TOP_PAGES = PAGES.filter((p) => p.id !== 'settings');
const page = ref('dashboard');

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
// { settings, browsers } from the Rust side, for the Settings page.
const settingsView = ref(null);
// Updates: `status` is the last check's answer.
const update = reactive({ status: null, checking: false, error: '', installing: false, progress: null });
let updateTimer = null;

const groupNames = computed(() => [...new Set(sites.value.map((s) => s.group_name).filter(Boolean))]);
const sessionList = computed(() => Object.values(sessions).sort((a, b) => a.site_name.localeCompare(b.site_name)));

function go(id) {
  form.value = null;
  page.value = id;
}

// `quiet`: the check at start-up. Being offline isn't worth an error
// there; the Check button shows it.
async function checkUpdate(quiet = false) {
  if (update.checking || update.installing) return;
  update.checking = true;
  if (!quiet) update.error = '';
  try {
    update.status = await api.checkUpdate();
    update.error = '';
  } catch (e) {
    if (!quiet) update.error = String(e);
  } finally {
    update.checking = false;
  }
}

async function installUpdate() {
  update.installing = true;
  update.error = '';
  update.progress = null;
  try {
    // On success the app restarts and this never comes back.
    await api.installUpdate();
  } catch (e) {
    update.error = String(e);
  }
  update.installing = false;
}

async function saveSettings(settings) {
  const saved = await api.saveSettings(settings);
  settingsView.value = { ...settingsView.value, settings: saved };
  return saved;
}

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
  stopListening.push(await api.onUpdateProgress((p) => (update.progress = p)));
  try {
    settingsView.value = await api.getSettings();
  } catch {
    // The Settings page stays on "Loading"; everything else works.
  }
  if (settingsView.value?.settings.check_updates) {
    checkUpdate(true);
    // And every six hours, for an app left open all week.
    updateTimer = setInterval(() => settingsView.value?.settings.check_updates && checkUpdate(true), 6 * 60 * 60 * 1000);
  }
  // Events only bring changes; start from the current list.
  try {
    for (const view of await api.listSessions()) sessions[view.site_id] = view;
  } catch {
    // The vault error banner already explains it.
  }
});

onUnmounted(() => {
  stopListening.forEach((stop) => stop());
  clearInterval(updateTimer);
});

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

// Renaming a group saves each of its sites with the new name. A missing
// password keeps the stored one.
async function renameGroup(from, to) {
  try {
    for (const site of sites.value.filter((s) => s.group_name === from)) {
      await api.updateSite(site.id, { ...site, group_name: to, password: null });
    }
    flash(`Renamed ${from} to ${to}`);
  } catch (e) {
    loadError.value = String(e);
  }
  await reload();
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
  <div class="shell">
    <nav class="sidebar" aria-label="Main">
      <div class="brand">NetLaunch</div>
      <button
        v-for="p in TOP_PAGES"
        :key="p.id"
        class="nav"
        :class="{ current: page === p.id }"
        :aria-current="page === p.id ? 'page' : undefined"
        @click="go(p.id)"
      >
        {{ p.label }}
        <span v-if="p.id === 'dashboard' && sessionList.length" class="count" title="Open windows">{{ sessionList.length }}</span>
      </button>

      <div class="spacer"></div>
      <button class="nav" :title="`Switch to ${theme === 'dark' ? 'light' : 'dark'} mode`" @click="toggleTheme">
        {{ theme === 'dark' ? 'Light mode' : 'Dark mode' }}
        <span aria-hidden="true">{{ theme === 'dark' ? '☀' : '☾' }}</span>
      </button>
      <button
        class="nav"
        :class="{ current: page === 'settings' }"
        :aria-current="page === 'settings' ? 'page' : undefined"
        @click="go('settings')"
      >
        Settings
        <span v-if="update.status?.available" class="dot" title="An update is available">Update</span>
      </button>
      <div class="version muted small">{{ settingsView ? `Version ${settingsView.version}` : '' }}</div>
    </nav>

    <main>
      <header class="page-head">
        <h1>{{ form ? (form.site ? 'Edit site' : 'Add site') : PAGES.find((p) => p.id === page).label }}</h1>
        <button v-if="status?.ok && !form && page !== 'settings'" class="primary" @click="form = { site: null }">Add site</button>
      </header>

      <p v-if="!status" class="muted">Opening the vault…</p>

      <div v-else-if="!status.ok && page !== 'settings'" class="banner error" role="alert">
        <strong>The vault couldn't be opened.</strong>
        <p>{{ status.error }}</p>
        <p v-if="status.path" class="muted">Vault file: <code>{{ status.path }}</code></p>
      </div>

      <SettingsPage
        v-else-if="page === 'settings' && !form"
        :view="settingsView"
        :save="saveSettings"
        :update="update"
        :vault-path="status.path"
        @check-update="checkUpdate()"
        @install-update="installUpdate"
      />

      <template v-else>
        <p v-if="notice && !form" class="banner ok" role="status">{{ notice }}</p>
        <p v-if="loadError" class="banner error" role="alert">{{ loadError }}</p>

        <SiteForm v-if="form" :site="form.site" :groups="groupNames" :recipes="recipes" :save="save" @cancel="form = null" />

        <template v-else-if="page === 'dashboard'">
          <div v-if="update.status?.available" class="banner ok update-banner" role="status">
            <span><strong>NetLaunch {{ update.status.available.version }} is available.</strong> You have {{ update.status.current }}.</span>
            <button @click="go('settings')">See the update</button>
          </div>

          <div class="tiles">
            <div class="tile"><strong>{{ sites.length }}</strong> {{ sites.length === 1 ? 'site' : 'sites' }}</div>
            <button class="tile" @click="go('groups')"><strong>{{ groupNames.length }}</strong> {{ groupNames.length === 1 ? 'group' : 'groups' }}</button>
            <div class="tile"><strong>{{ sessionList.length }}</strong> open {{ sessionList.length === 1 ? 'window' : 'windows' }}</div>
          </div>

          <SessionPanel
            v-if="sessionList.length"
            :sessions="sessionList"
            :pending="pending"
            @release="(v) => windowAction(v, 'release')"
            @close="(v) => windowAction(v, 'close')"
            @retry="(v) => windowAction(v, 'retry')"
          />

          <SiteList
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

        <SiteList
          v-else
          grouped
          :sites="sites"
          :logins="logins"
          :sessions="sessions"
          @add="form = { site: null }"
          @open="open"
          @trust="trustAndOpen"
          @edit="(site) => (form = { site })"
          @delete="remove"
          @rename-group="renameGroup"
        />
      </template>
    </main>
  </div>
</template>
