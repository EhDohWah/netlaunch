<script setup>
// Top level: loads the vault status and the site list, and switches
// between the list and the add/edit form.
import { onMounted, ref } from 'vue';
import * as api from './api.js';
import SiteList from './components/SiteList.vue';
import SiteForm from './components/SiteForm.vue';

const status = ref(null);
const sites = ref([]);
const loadError = ref('');
// null = the list; { site: null } = adding; { site } = editing.
const form = ref(null);
const notice = ref('');

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
});

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

      <SiteForm
        v-if="form"
        :site="form.site"
        :groups="[...new Set(sites.map((s) => s.group_name).filter(Boolean))]"
        :save="save"
        @cancel="form = null"
      />
      <SiteList v-else :sites="sites" @add="form = { site: null }" @edit="(site) => (form = { site })" @delete="remove" />
    </template>
  </main>

  <footer v-if="status?.ok && status.path" class="muted footer">Vault: <code>{{ status.path }}</code></footer>
</template>
