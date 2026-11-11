<script setup>
// Top level: the desktop shell. A command bar on top, a navigation pane
// on the left (Dashboard, Groups with one entry per group, Settings), the
// site grid with the "Open windows" dock in the middle, and a status bar.
//
// It loads the vault status and the site list, keeps the open windows up
// to date, and looks for updates. Nothing here holds a password.
import { computed, nextTick, onMounted, onUnmounted, reactive, ref } from 'vue';
import * as api from './api.js';
import ContextMenu from './components/ContextMenu.vue';
import Icon from './components/Icon.vue';
import Modal from './components/Modal.vue';
import SettingsPage from './components/SettingsPage.vue';
import SiteForm from './components/SiteForm.vue';
import SiteGrid from './components/SiteGrid.vue';
import WindowDock from './components/WindowDock.vue';
import { isBusy, siteStatus } from './states.js';
import { theme, toggleTheme } from './theme.js';

// Where the admin is: the page, and on Groups the chosen group
// (null = every group, '' = the sites without one).
const page = ref('dashboard');
const group = ref(null);

const status = ref(null);
const sites = ref([]);
const loadError = ref('');
// null = no form; { site: null } = adding; { site } = editing.
const form = ref(null);
const notice = ref('');
const recipes = ref([]);
// The result of each site's last Open: { busy } while running, then
// { outcome, message, warnings, fix }.
const logins = reactive({});
// Open windows by site id: SessionView, kept up to date by events.
const sessions = reactive({});
// Site ids with a Release/Close/Retry running.
const pending = reactive({});
const stopListening = [];
// { settings, browsers, version, can_update } from the Rust side.
const settingsView = ref(null);
// Updates: `status` is the last check's answer.
const update = reactive({ status: null, checking: false, error: '', installing: false, progress: null });
let updateTimer = null;

const search = ref('');
const searchBox = ref(null);
const grid = ref(null);
const selectedId = ref(null);
// The right-click menu: { x, y, items } or null.
const menu = ref(null);
// A small dialog: { kind: 'delete', site } or { kind: 'rename', from, to }.
const dialog = ref(null);

// The navigation pane folds to icons; remembered like the theme.
const NAV_KEY = 'netlaunch-nav-collapsed';
const collapsed = ref(readFlag(NAV_KEY));
const groupsOpen = ref(true);
function readFlag(key) {
  try {
    return localStorage.getItem(key) === '1';
  } catch {
    return false;
  }
}
function toggleNav() {
  collapsed.value = !collapsed.value;
  try {
    localStorage.setItem(NAV_KEY, collapsed.value ? '1' : '0');
  } catch {
    // Not saved; it applies until the app closes.
  }
}

const groupNames = computed(() => [...new Set(sites.value.map((s) => s.group_name).filter(Boolean))].sort((a, b) => a.localeCompare(b)));
const ungrouped = computed(() => sites.value.filter((s) => !s.group_name).length);
const countIn = (name) => sites.value.filter((s) => (s.group_name || '') === name).length;
const sessionList = computed(() => Object.values(sessions).sort((a, b) => a.site_name.localeCompare(b.site_name)));

// The grid's rows: the chosen group, narrowed by the search box.
const visibleSites = computed(() => {
  let list = sites.value;
  if (page.value === 'groups' && group.value !== null) list = list.filter((s) => (s.group_name || '') === group.value);
  const q = search.value.trim().toLowerCase();
  if (!q) return list;
  return list.filter((s) => [s.name, s.group_name, s.username, s.login_url].some((field) => field && field.toLowerCase().includes(q)));
});

const selected = computed(() => sites.value.find((s) => s.id === selectedId.value) ?? null);
const selectedSession = computed(() => (selected.value ? sessions[selected.value.id] ?? null : null));
const selectedStatus = computed(() => (selected.value ? siteStatus(selected.value, sessions, logins) : null));
const canOpen = computed(() => !!selected.value && !logins[selected.value.id]?.busy && !selectedSession.value);
const windowBusy = computed(() => !selectedSession.value || isBusy(selectedSession.value.state) || !!pending[selectedSession.value.site_id]);

const title = computed(() => {
  if (form.value) return form.value.site ? 'Edit site' : 'Add site';
  if (page.value === 'settings') return 'Settings';
  if (page.value === 'dashboard') return 'All sites';
  return group.value === null ? 'Groups' : group.value || 'Ungrouped';
});
const BROWSERS = { edge: 'Microsoft Edge', chrome: 'Google Chrome', custom: 'Custom browser' };

function go(id, groupName = null) {
  form.value = null;
  page.value = id;
  group.value = groupName;
  nextTick(() => grid.value?.focus());
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

// App-wide shortcuts. The grid handles its own (Enter, Del, F2, arrows).
function shortcut(event) {
  if (dialog.value || !status.value?.ok) return;
  if (!event.ctrlKey || event.altKey) return;
  const key = event.key.toLowerCase();
  if (key === 'n' && !form.value) {
    event.preventDefault();
    form.value = { site: null };
  } else if (key === 'f' && !form.value && page.value !== 'settings') {
    event.preventDefault();
    searchBox.value?.focus();
    searchBox.value?.select();
  }
}

onMounted(async () => {
  window.addEventListener('keydown', shortcut);
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
    // The vault error bar already explains it.
  }
  grid.value?.focus();
});

onUnmounted(() => {
  window.removeEventListener('keydown', shortcut);
  stopListening.forEach((stop) => stop());
  clearInterval(updateTimer);
});

async function open(site) {
  if (logins[site.id]?.busy || sessions[site.id]) return;
  selectedId.value = site.id;
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

// Release, Close and Retry. Errors show as the site's status.
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

// A line in the status bar for a few seconds.
function flash(text) {
  notice.value = text;
  setTimeout(() => {
    if (notice.value === text) notice.value = '';
  }, 4000);
}

// SiteForm hands over the fields; we save and return to the list. Errors
// are thrown back so the form can show them next to the fields.
async function save(fields) {
  const editing = form.value.site;
  const saved = editing ? await api.updateSite(editing.id, fields) : await api.addSite(fields);
  form.value = null;
  await reload();
  selectedId.value = saved.id;
  flash(`${editing ? 'Saved' : 'Added'} ${saved.name}`);
}

// Changing a site's group, or renaming a group, saves each site with the
// new name. A missing password keeps the stored one.
async function setGroup(list, to, done) {
  try {
    for (const site of list) await api.updateSite(site.id, { ...site, group_name: to, password: null });
    flash(done);
  } catch (e) {
    loadError.value = String(e);
  }
  await reload();
}

async function finishRename() {
  const { from, to } = dialog.value;
  dialog.value = null;
  const name = to.trim();
  if (!name || name === from) return;
  await setGroup(sites.value.filter((s) => s.group_name === from), name, `Renamed ${from} to ${name}`);
  if (group.value === from) group.value = name;
}

async function remove() {
  const { site } = dialog.value;
  dialog.value = null;
  try {
    await api.deleteSite(site.id);
    if (selectedId.value === site.id) selectedId.value = null;
    await reload();
    flash(`Deleted ${site.name}`);
  } catch (e) {
    loadError.value = String(e);
  }
}

async function copyUrl(site) {
  try {
    await navigator.clipboard.writeText(site.login_url);
    flash('Copied the URL');
  } catch {
    flash("The URL couldn't be copied");
  }
}

function siteMenu({ site, x, y }) {
  const view = sessions[site.id];
  const busy = !view || isBusy(view.state) || !!pending[site.id];
  const items = [
    { label: 'Open', icon: 'open', hint: 'Enter', disabled: !!logins[site.id]?.busy || !!view, action: () => open(site) },
    { label: 'Edit', icon: 'edit', hint: 'F2', action: () => (form.value = { site }) },
    { label: 'Copy URL', icon: 'copy', action: () => copyUrl(site) },
    {
      label: 'Move to group',
      icon: 'move',
      children: [
        ...groupNames.value.filter((g) => g !== site.group_name).map((g) => ({ label: g, action: () => setGroup([site], g, `Moved ${site.name} to ${g}`) })),
        { label: 'No group', disabled: !site.group_name, action: () => setGroup([site], null, `Removed ${site.name} from its group`) },
      ],
    },
  ];
  if (view) {
    items.push({ separator: true });
    if (view.window_mode === 'managed') items.push({ label: 'Release window', icon: 'release', disabled: busy, action: () => windowAction(view, 'release') });
    items.push({ label: 'Close window', icon: 'dismiss', disabled: busy, action: () => windowAction(view, 'close') });
    items.push({ label: 'Retry login', icon: 'retry', disabled: busy, action: () => windowAction(view, 'retry') });
  }
  items.push({ separator: true }, { label: 'Delete', icon: 'delete', hint: 'Del', action: () => (dialog.value = { kind: 'delete', site }) });
  menu.value = { x, y, items };
}

function groupMenu({ group: name, x, y }) {
  menu.value = {
    x,
    y,
    items: [
      { label: 'Show only this group', icon: 'folder', action: () => go('groups', name) },
      { label: 'Rename group', icon: 'rename', disabled: !name, action: () => (dialog.value = { kind: 'rename', from: name, to: name }) },
    ],
  };
}
</script>

<template>
  <div class="app" :class="{ collapsed }">
    <header class="cmdbar">
      <button class="btn icon-only" :aria-label="collapsed ? 'Expand navigation' : 'Collapse navigation'" :aria-expanded="!collapsed" @click="toggleNav">
        <Icon name="menu" />
      </button>
      <strong class="brand">NetLaunch</strong>

      <template v-if="status?.ok && !form && page !== 'settings'">
        <button class="btn accent" title="Add a site (Ctrl+N)" @click="form = { site: null }"><Icon name="add" />Add</button>
        <button class="btn" :disabled="!canOpen" title="Open the selected site (Enter)" @click="open(selected)"><Icon name="open" />Open</button>
        <button class="btn" :disabled="!selected" title="Edit the selected site (F2)" @click="form = { site: selected }"><Icon name="edit" />Edit</button>
        <button class="btn" :disabled="!selected" title="Delete the selected site (Del)" @click="dialog = { kind: 'delete', site: selected }"><Icon name="delete" />Delete</button>
        <span class="sep"></span>
        <button
          v-if="selectedSession?.window_mode !== 'released'"
          class="btn"
          :disabled="windowBusy"
          title="Disconnect NetLaunch and leave the selected site's window open"
          @click="windowAction(selectedSession, 'release')"
        >
          <Icon name="release" />Release
        </button>
        <button class="btn" :disabled="windowBusy" title="Close the selected site's window" @click="windowAction(selectedSession, 'close')"><Icon name="dismiss" />Close window</button>
        <button class="btn" :disabled="windowBusy" title="Close the window and log in again" @click="windowAction(selectedSession, 'retry')"><Icon name="retry" />Retry</button>
        <template v-if="page === 'groups' && group">
          <span class="sep"></span>
          <button class="btn" @click="dialog = { kind: 'rename', from: group, to: group }"><Icon name="rename" />Rename group</button>
        </template>
        <span class="grow"></span>
        <label class="searchbox">
          <Icon name="search" />
          <input ref="searchBox" v-model="search" type="search" placeholder="Search sites (Ctrl+F)" aria-label="Search sites" @keydown.enter="visibleSites[0] && open(visibleSites[0])" @keydown.esc="search = ''" />
        </label>
      </template>
    </header>

    <div class="body">
      <nav class="navpane" aria-label="Main">
        <button class="nav" :class="{ current: page === 'dashboard' }" :aria-current="page === 'dashboard' ? 'page' : undefined" title="Dashboard" @click="go('dashboard')">
          <Icon name="home" /><span class="label">Dashboard</span>
          <span v-if="sessionList.length" class="count" title="Open windows">{{ sessionList.length }}</span>
        </button>
        <div class="nav-row">
          <button class="nav" :class="{ current: page === 'groups' && group === null }" :aria-current="page === 'groups' && group === null ? 'page' : undefined" title="Groups" @click="go('groups')">
            <Icon name="folder" /><span class="label">Groups</span>
          </button>
          <button class="btn icon-only tree-toggle" :aria-label="groupsOpen ? 'Hide the groups' : 'Show the groups'" :aria-expanded="groupsOpen" @click="groupsOpen = !groupsOpen">
            <Icon :name="groupsOpen ? 'down' : 'right'" />
          </button>
        </div>
        <div v-if="groupsOpen && !collapsed" class="tree">
          <button v-for="g in groupNames" :key="g" class="nav leaf" :class="{ current: page === 'groups' && group === g }" @click="go('groups', g)" @contextmenu.prevent="groupMenu({ group: g, x: $event.clientX, y: $event.clientY })">
            <span class="label">{{ g }}</span><span class="count">{{ countIn(g) }}</span>
          </button>
          <button v-if="ungrouped" class="nav leaf" :class="{ current: page === 'groups' && group === '' }" @click="go('groups', '')">
            <span class="label">Ungrouped</span><span class="count">{{ ungrouped }}</span>
          </button>
        </div>

        <span class="grow"></span>
        <button class="nav" :title="`Switch to ${theme === 'dark' ? 'light' : 'dark'} mode`" @click="toggleTheme">
          <Icon :name="theme === 'dark' ? 'sun' : 'moon'" /><span class="label">{{ theme === 'dark' ? 'Light mode' : 'Dark mode' }}</span>
        </button>
        <button class="nav" :class="{ current: page === 'settings' }" :aria-current="page === 'settings' ? 'page' : undefined" title="Settings" @click="go('settings')">
          <Icon name="settings" /><span class="label">Settings</span>
          <span v-if="update.status?.available" class="count update" title="An update is available">Update</span>
        </button>
      </nav>

      <main class="content">
        <p v-if="!status" class="pad muted">Opening the vault…</p>

        <div v-else-if="!status.ok && page !== 'settings'" class="infobar error" role="alert">
          <span><strong>The vault couldn't be opened.</strong> {{ status.error }}</span>
          <span v-if="status.path" class="dim">Vault file: <code>{{ status.path }}</code></span>
        </div>

        <div v-else-if="page === 'settings' && !form" class="scroll pad">
          <SettingsPage :view="settingsView" :save="saveSettings" :update="update" :vault-path="status.path" @check-update="checkUpdate()" @install-update="installUpdate" />
        </div>

        <div v-else-if="form" class="scroll pad">
          <SiteForm :site="form.site" :groups="groupNames" :recipes="recipes" :save="save" @cancel="form = null" />
        </div>

        <template v-else>
          <div v-if="update.status?.available" class="infobar ok" role="status">
            <span><strong>NetLaunch {{ update.status.available.version }} is available.</strong> You have {{ update.status.current }}.</span>
            <button class="btn small" @click="go('settings')">See the update</button>
          </div>
          <p v-if="loadError" class="infobar error" role="alert">{{ loadError }}</p>
          <!-- What the selected site's last login said, when it needs reading. -->
          <div v-if="selectedStatus && (selectedStatus.message || selectedStatus.warnings.length)" class="infobar" :class="selectedStatus.kind" role="status">
            <span>
              <strong>{{ selected.name }}: {{ selectedStatus.label }}.</strong> {{ selectedStatus.message }}
              <span v-for="w in selectedStatus.warnings" :key="w" class="warning"> Warning: {{ w }}</span>
            </span>
            <button v-if="selectedStatus.fix === 'trust_certificate'" class="btn small" title="Only for devices on your own network" @click="trustAndOpen(selected)">
              Trust this certificate and open again
            </button>
          </div>

          <section v-if="!sites.length" class="empty">
            <h2>No sites yet</h2>
            <p class="muted">Add the admin consoles you log in to. Passwords are encrypted and stay on this PC.</p>
            <button class="btn accent" @click="form = { site: null }"><Icon name="add" />Add your first site</button>
          </section>
          <SiteGrid
            v-else
            ref="grid"
            :sites="visibleSites"
            :logins="logins"
            :sessions="sessions"
            :selected-id="selectedId"
            :grouped="page === 'groups' && group === null"
            @select="(id) => (selectedId = id)"
            @open="open"
            @edit="(site) => (form = { site })"
            @delete="(site) => (dialog = { kind: 'delete', site })"
            @menu="siteMenu"
            @group-menu="groupMenu"
          />

          <WindowDock
            v-if="sessionList.length"
            :sessions="sessionList"
            :pending="pending"
            @select="(id) => (selectedId = id)"
            @release="(v) => windowAction(v, 'release')"
            @close="(v) => windowAction(v, 'close')"
            @retry="(v) => windowAction(v, 'retry')"
          />
        </template>
      </main>
    </div>

    <footer class="statusbar">
      <span>{{ sites.length }} {{ sites.length === 1 ? 'site' : 'sites' }}</span>
      <span>{{ groupNames.length }} {{ groupNames.length === 1 ? 'group' : 'groups' }}</span>
      <span>{{ sessionList.length }} open {{ sessionList.length === 1 ? 'window' : 'windows' }}</span>
      <span v-if="!form && page !== 'settings'">{{ title }}<template v-if="selected">: {{ selected.name }} selected</template></span>
      <span v-if="notice" class="notice" role="status">{{ notice }}</span>
      <span class="grow"></span>
      <span v-if="settingsView">{{ BROWSERS[settingsView.settings.browser] }}</span>
      <span v-if="settingsView">NetLaunch {{ settingsView.version }}</span>
    </footer>

    <ContextMenu v-if="menu" :x="menu.x" :y="menu.y" :items="menu.items" @close="menu = null" />

    <Modal v-if="dialog?.kind === 'delete'" :title="`Delete ${dialog.site.name}?`" @cancel="dialog = null" @submit="remove">
      <p>Its saved login is removed from this PC. If its window is open, it is closed first.</p>
      <template #buttons>
        <button type="submit" class="btn danger">Delete</button>
        <button type="button" class="btn outline" @click="dialog = null">Cancel</button>
      </template>
    </Modal>

    <Modal v-if="dialog?.kind === 'rename'" :title="`Rename ${dialog.from}`" @cancel="dialog = null" @submit="finishRename">
      <label class="field">
        <span>Group name</span>
        <input v-model="dialog.to" required />
      </label>
      <template #buttons>
        <button type="submit" class="btn accent">Rename</button>
        <button type="button" class="btn outline" @click="dialog = null">Cancel</button>
      </template>
    </Modal>
  </div>
</template>
