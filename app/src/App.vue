<script setup>
// Top level: the two-pane window.
//   Left:  search, Add, the All / Open now choice, the site list, and at
//          the bottom Settings and the theme toggle.
//   Right: the selected site's details, or Settings.
//
// It loads the vault status and the site list, keeps the open windows up
// to date, and looks for updates. Nothing here holds a password.
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from 'vue';
import * as api from './api.js';
import ContextMenu from './components/ContextMenu.vue';
import Icon from './components/Icon.vue';
import Modal from './components/Modal.vue';
import SettingsPage from './components/SettingsPage.vue';
import SiteDetails from './components/SiteDetails.vue';
import SiteForm from './components/SiteForm.vue';
import SiteList from './components/SiteList.vue';
import { isBusy, siteStatus } from './states.js';
import { theme, toggleTheme } from './theme.js';

// The right pane: 'site' (the selected site) or 'settings'.
const page = ref('site');
// The list shows 'all' sites or only those with a window 'open'.
const filter = ref('all');

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
// { settings, browsers, version, can_update, mica_supported } from the Rust side.
const settingsView = ref(null);
// Updates: `status` is the last check's answer.
const update = reactive({ status: null, checking: false, error: '', installing: false, progress: null });
let updateTimer = null;

const search = ref('');
const searchBox = ref(null);
const list = ref(null);
const selectedId = ref(null);
// The right-click menu: { x, y, items } or null.
const menu = ref(null);
// A small dialog, by `kind`:
//   delete   { site }             confirm deleting a site
//   rename   { from, to, error }  rename a group
//   ungroup  { name }             confirm removing a group (its sites are kept)
//   newgroup { site, to, error }  move a site to a group that doesn't exist yet
const dialog = ref(null);
// Read out by screen readers when it changes (see the template's last lines).
const announcement = ref('');

// One entry per group, whatever the capitals (the vault keeps one
// spelling per group; older data may not).
const groupNames = computed(() => {
  const seen = new Map();
  for (const site of sites.value) {
    if (site.group_name && !seen.has(site.group_name.toLowerCase())) seen.set(site.group_name.toLowerCase(), site.group_name);
  }
  return [...seen.values()].sort((a, b) => a.localeCompare(b));
});
const sameName = (a, b) => (a ?? '').trim().toLowerCase() === (b ?? '').trim().toLowerCase();
const sitesIn = (name) => sites.value.filter((s) => sameName(s.group_name, name));
// While renaming: the other group that already has the typed name, if
// any. Renaming onto it merges the two.
const mergeInto = computed(() => {
  if (dialog.value?.kind !== 'rename' || sameName(dialog.value.to, dialog.value.from)) return null;
  return groupNames.value.find((g) => sameName(g, dialog.value.to)) ?? null;
});
const openCount = computed(() => Object.keys(sessions).length);

// The list's rows: the chosen filter, narrowed by the search box.
const visibleSites = computed(() => {
  let rows = sites.value;
  if (filter.value === 'open') rows = rows.filter((s) => sessions[s.id]);
  const q = search.value.trim().toLowerCase();
  if (!q) return rows;
  return rows.filter((s) => [s.name, s.group_name, s.username, s.login_url].some((field) => field && field.toLowerCase().includes(q)));
});
// Group headings only make sense for the whole, unsearched list.
const grouped = computed(() => filter.value === 'all' && !search.value.trim());
const emptyText = computed(() => {
  if (search.value.trim()) return `No sites match “${search.value.trim()}”.`;
  if (filter.value === 'open') return 'No windows are open. Open a site and it appears here.';
  return 'No sites yet.';
});

const selected = computed(() => sites.value.find((s) => s.id === selectedId.value) ?? null);
const selectedStatus = computed(() => (selected.value ? siteStatus(selected.value, sessions, logins) : null));

// Tell screen-reader users when the selected site's status changes, as
// sighted users see it change in the details pane.
watch(
  () => selectedStatus.value && `${selected.value.name}: ${selectedStatus.value.label}. ${selectedStatus.value.message}`,
  (text) => {
    if (text) announcement.value = text;
  },
);

function showSettings() {
  page.value = 'settings';
}

function select(id) {
  selectedId.value = id;
  page.value = 'site';
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
  await applyMica();
  return saved;
}

// Mica shows through a see-through page, so the page only drops its own
// background once Windows confirms the material is on.
async function applyMica() {
  let on = false;
  try {
    on = await api.setMica(!!settingsView.value?.settings.mica);
  } catch {
    // Left off.
  }
  if (on) document.documentElement.dataset.mica = 'on';
  else delete document.documentElement.dataset.mica;
}

async function reload() {
  loadError.value = '';
  try {
    sites.value = await api.listSites();
  } catch (e) {
    loadError.value = String(e);
  }
  // Start with a site showing rather than an empty pane.
  if (!selected.value && sites.value.length) selectedId.value = sites.value[0].id;
}

// App-wide shortcuts. The list handles its own (Enter, Del, F2, arrows).
function shortcut(event) {
  if (dialog.value || form.value || !status.value?.ok) return;
  if (!event.ctrlKey || event.altKey) return;
  const key = event.key.toLowerCase();
  if (key === 'n') {
    event.preventDefault();
    form.value = { site: null };
  } else if (key === 'f') {
    event.preventDefault();
    searchBox.value?.focus();
    searchBox.value?.select();
  }
}

// In the search box: Enter opens the first match, Down moves into the
// list, Esc clears the search.
function searchKey(event) {
  if (event.key === 'Enter' && visibleSites.value[0]) {
    open(visibleSites.value[0]);
  } else if (event.key === 'ArrowDown') {
    if (visibleSites.value[0] && !visibleSites.value.some((s) => s.id === selectedId.value)) select(visibleSites.value[0].id);
    list.value?.focus();
  } else if (event.key === 'Escape' && search.value) {
    search.value = '';
  } else {
    return;
  }
  event.preventDefault();
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
  applyMica();
  if (settingsView.value?.settings.check_updates) {
    checkUpdate(true);
    // And every six hours, for an app left open all week.
    updateTimer = setInterval(() => settingsView.value?.settings.check_updates && checkUpdate(true), 6 * 60 * 60 * 1000);
  }
  // Events only bring changes; start from the current list.
  try {
    for (const view of await api.listSessions()) sessions[view.site_id] = view;
  } catch {
    // The vault error message already explains it.
  }
  list.value?.focus();
});

onUnmounted(() => {
  window.removeEventListener('keydown', shortcut);
  stopListening.forEach((stop) => stop());
  clearInterval(updateTimer);
});

async function open(site) {
  if (logins[site.id]?.busy || sessions[site.id]) return;
  select(site.id);
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

// The "Trust this certificate" button: turn on the site's self-signed
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

// A line at the bottom of the list for a few seconds, also read out.
function flash(text) {
  notice.value = text;
  announcement.value = text;
  setTimeout(() => {
    if (notice.value === text) notice.value = '';
  }, 4000);
}

// SiteForm hands over the fields; we save and close the dialog. Errors
// are thrown back so the form can show them.
async function save(fields) {
  const editing = form.value.site;
  const saved = editing ? await api.updateSite(editing.id, fields) : await api.addSite(fields);
  form.value = null;
  await reload();
  select(saved.id);
  flash(`${editing ? 'Saved' : 'Added'} ${saved.name}`);
}

// Moves one site to a group (null for none). A missing password keeps
// the stored one. Throws with the vault's message on failure.
async function moveToGroup(site, to, done) {
  await api.updateSite(site.id, { ...site, group_name: to, password: null });
  await reload();
  flash(done);
}
function moveFromMenu(site, to, done) {
  moveToGroup(site, to, done).catch((e) => (loadError.value = String(e)));
}

// Group names never start with "invalid input: " for the admin.
const plain = (e) => String(e).replace(/^invalid input: /, '');

async function finishRename() {
  const { from, to } = dialog.value;
  const name = to.trim();
  if (!name || name === from) return (dialog.value = null);
  const merged = mergeInto.value;
  try {
    // One step in the vault: every site in the group changes, or none.
    await api.renameGroup(from, name);
  } catch (e) {
    dialog.value.error = plain(e);
    return;
  }
  dialog.value = null;
  list.value?.groupRenamed(from, merged ?? name);
  await reload();
  flash(merged ? `Merged ${from} into ${merged}` : `Renamed ${from} to ${name}`);
}

async function finishUngroup() {
  const { name } = dialog.value;
  dialog.value = null;
  try {
    const moved = await api.removeGroup(name);
    await reload();
    flash(`Removed the group ${name}. ${moved} ${moved === 1 ? 'site is' : 'sites are'} now under No group.`);
  } catch (e) {
    loadError.value = plain(e);
  }
}

async function finishNewGroup() {
  const { site, to } = dialog.value;
  const name = to.trim();
  if (!name) return (dialog.value = null);
  try {
    await moveToGroup(site, name, `Moved ${site.name} to ${name}`);
    dialog.value = null;
  } catch (e) {
    dialog.value.error = plain(e);
  }
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
        ...groupNames.value.filter((g) => !sameName(g, site.group_name)).map((g) => ({ label: g, action: () => moveFromMenu(site, g, `Moved ${site.name} to ${g}`) })),
        { label: 'No group', disabled: !site.group_name, action: () => moveFromMenu(site, null, `Removed ${site.name} from its group`) },
        { label: 'New group…', action: () => (dialog.value = { kind: 'newgroup', site, to: '', error: '' }) },
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

// "No group" is a heading, not a group: there is nothing to rename or remove.
function renameGroup(name) {
  if (name) dialog.value = { kind: 'rename', from: name, to: name, error: '' };
}
function removeGroup(name) {
  if (name) dialog.value = { kind: 'ungroup', name };
}

function groupMenu({ group: name, x, y }) {
  menu.value = {
    x,
    y,
    items: [
      { label: 'Rename group', icon: 'rename', hint: 'F2', disabled: !name, action: () => renameGroup(name) },
      { label: 'Remove group…', icon: 'dismiss', hint: 'Del', disabled: !name, action: () => removeGroup(name) },
    ],
  };
}

// The keyboard goes back to the list when the menu closes, unless the
// chosen item opened a dialog (which takes the focus itself).
function closeMenu() {
  menu.value = null;
  nextTick(() => {
    if (!dialog.value && !form.value) list.value?.focus();
  });
}
</script>

<template>
  <div class="app">
    <aside class="listpane" aria-label="Sites">
      <div class="list-top">
        <label class="searchbox">
          <Icon name="search" />
          <input ref="searchBox" v-model="search" type="search" autocomplete="off" spellcheck="false" placeholder="Search sites…" aria-label="Search sites" title="Search (Ctrl+F)" @keydown="searchKey" />
        </label>
        <button class="btn outline icon-only" :disabled="!status?.ok" aria-label="Add site" title="Add site (Ctrl+N)" @click="form = { site: null }">
          <Icon name="add" />
        </button>
      </div>

      <div class="selector">
        <button :aria-pressed="filter === 'all'" @click="filter = 'all'">All sites <span class="count">{{ sites.length }}</span></button>
        <button :aria-pressed="filter === 'open'" @click="filter = 'open'">Open now <span class="count">{{ openCount }}</span></button>
      </div>

      <SiteList
        ref="list"
        :sites="visibleSites"
        :logins="logins"
        :sessions="sessions"
        :selected-id="page === 'site' ? selectedId : null"
        :grouped="grouped"
        :empty-text="emptyText"
        @select="select"
        @open="open"
        @edit="(site) => (form = { site })"
        @delete="(site) => (dialog = { kind: 'delete', site })"
        @menu="siteMenu"
        @group-menu="groupMenu"
        @rename-group="renameGroup"
        @remove-group="removeGroup"
      />

      <div class="list-foot">
        <button class="btn" :class="{ current: page === 'settings' }" :aria-current="page === 'settings' ? 'page' : undefined" @click="showSettings">
          <Icon name="settings" />Settings
          <span v-if="update.status?.available" class="pill" title="An update is available">Update</span>
        </button>
        <button class="btn icon-only" :aria-label="`Switch to ${theme === 'dark' ? 'light' : 'dark'} mode`" :title="`Switch to ${theme === 'dark' ? 'light' : 'dark'} mode`" @click="toggleTheme">
          <Icon :name="theme === 'dark' ? 'sun' : 'moon'" />
        </button>
        <span class="grow"></span>
        <span class="foot-text">{{ notice || `${sites.length} ${sites.length === 1 ? 'site' : 'sites'} · ${openCount} open` }}</span>
      </div>
    </aside>

    <main class="pane">
      <div v-if="update.status?.available && page !== 'settings'" class="infobar ok">
        <span><strong>NetLaunch {{ update.status.available.version }} is available.</strong> You have {{ update.status.current }}.</span>
        <button class="btn outline" @click="showSettings">See the update</button>
      </div>
      <p v-if="loadError" class="infobar error" role="alert">{{ loadError }}</p>

      <div class="pane-body">
        <p v-if="!status" class="muted">Opening the vault…</p>

        <SettingsPage
          v-else-if="page === 'settings'"
          :view="settingsView"
          :save="saveSettings"
          :update="update"
          :vault-path="status.path"
          @check-update="checkUpdate()"
          @install-update="installUpdate"
        />

        <div v-else-if="!status.ok" class="blank" role="alert">
          <h1>The vault couldn't be opened</h1>
          <p>{{ status.error }}</p>
          <p v-if="status.path" class="muted">Vault file: <code>{{ status.path }}</code></p>
        </div>

        <SiteDetails
          v-else-if="selected"
          :site="selected"
          :status="selectedStatus"
          :session="sessions[selected.id] ?? null"
          :opening="!!logins[selected.id]?.busy"
          :pending="!!pending[selected.id]"
          :recipes="recipes"
          @open="open(selected)"
          @edit="form = { site: selected }"
          @copy="copyUrl(selected)"
          @delete="dialog = { kind: 'delete', site: selected }"
          @trust="trustAndOpen(selected)"
          @release="windowAction(sessions[selected.id], 'release')"
          @close="windowAction(sessions[selected.id], 'close')"
          @retry="windowAction(sessions[selected.id], 'retry')"
        />

        <div v-else class="blank">
          <template v-if="!sites.length">
            <h1>Add your first site</h1>
            <p class="muted">Save the admin pages you log in to. Passwords are encrypted and stay on this PC.</p>
            <button class="btn accent" @click="form = { site: null }"><Icon name="add" />Add site</button>
          </template>
          <p v-else class="muted">Select a site to see its details.</p>
        </div>
      </div>
    </main>

    <ContextMenu v-if="menu" :x="menu.x" :y="menu.y" :items="menu.items" @close="closeMenu" />

    <!-- `key`: a fresh form for each site, never the last one's fields. -->
    <SiteForm v-if="form" :key="form.site?.id ?? 'new'" :site="form.site" :groups="groupNames" :recipes="recipes" :save="save" @cancel="form = null" />

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
        <span>
          <input v-model="dialog.to" :class="{ invalid: dialog.error }" maxlength="40" autocomplete="off" />
          <span v-if="dialog.error" class="problem" role="alert">{{ dialog.error }}</span>
          <span v-else-if="mergeInto" class="hint">{{ mergeInto }} already exists. Its sites and these become one group.</span>
        </span>
      </label>
      <template #buttons>
        <button type="submit" class="btn accent">{{ mergeInto ? 'Merge groups' : 'Rename' }}</button>
        <button type="button" class="btn outline" @click="dialog = null">Cancel</button>
      </template>
    </Modal>

    <Modal v-if="dialog?.kind === 'ungroup'" :title="`Remove the group ${dialog.name}?`" @cancel="dialog = null" @submit="finishUngroup">
      <p>
        {{ sitesIn(dialog.name).length === 1 ? 'Its site is kept and moves' : `Its ${sitesIn(dialog.name).length} sites are kept and move` }}
        to No group. Saved logins and open windows are not affected.
      </p>
      <template #buttons>
        <button type="submit" class="btn accent">Remove group</button>
        <button type="button" class="btn outline" @click="dialog = null">Cancel</button>
      </template>
    </Modal>

    <Modal v-if="dialog?.kind === 'newgroup'" :title="`Move ${dialog.site.name} to a new group`" @cancel="dialog = null" @submit="finishNewGroup">
      <label class="field">
        <span>Group name</span>
        <span>
          <input v-model="dialog.to" :class="{ invalid: dialog.error }" maxlength="40" autocomplete="off" placeholder="Switches…" />
          <span v-if="dialog.error" class="problem" role="alert">{{ dialog.error }}</span>
        </span>
      </label>
      <template #buttons>
        <button type="submit" class="btn accent">Move</button>
        <button type="button" class="btn outline" @click="dialog = null">Cancel</button>
      </template>
    </Modal>

    <!-- Always in the page, so screen readers notice when its text
         changes. Hidden from sight: the same text is shown elsewhere. -->
    <div class="sr-only" aria-live="polite">{{ announcement }}</div>
  </div>
</template>
