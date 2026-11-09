<script setup>
// The home screen: sites grouped by group name, a search box, and
// Open / Edit / Delete per site. Open arrives with the login engine (M5).
import { computed, ref } from 'vue';

const props = defineProps({ sites: { type: Array, required: true } });
const emit = defineEmits(['add', 'edit', 'delete']);

const search = ref('');
// The id of the site whose Delete button was clicked once and is waiting
// for confirmation.
const confirming = ref(null);

const filtered = computed(() => {
  const q = search.value.trim().toLowerCase();
  if (!q) return props.sites;
  return props.sites.filter((s) =>
    [s.name, s.group_name, s.username, s.login_url].some((field) => field && field.toLowerCase().includes(q)),
  );
});

// The vault already sorts by group, then name, with ungrouped sites last,
// so grouping only has to split the list where the group changes.
const groups = computed(() => {
  const result = [];
  for (const site of filtered.value) {
    const name = site.group_name || 'Ungrouped';
    if (!result.length || result[result.length - 1].name !== name) result.push({ name, sites: [] });
    result[result.length - 1].sites.push(site);
  }
  return result;
});

function lastUsed(site) {
  return site.last_used_at ? new Date(site.last_used_at).toLocaleString() : 'Never';
}

function confirmDelete(site) {
  confirming.value = null;
  emit('delete', site);
}
</script>

<template>
  <section v-if="!sites.length" class="empty">
    <h2>No sites yet</h2>
    <p class="muted">Add the admin consoles you log in to. Passwords are encrypted and stay on this PC.</p>
    <button class="primary" @click="emit('add')">Add your first site</button>
  </section>

  <template v-else>
    <input v-model="search" class="search" type="search" placeholder="Search by name, group, username or URL" aria-label="Search sites" />

    <p v-if="!groups.length" class="muted">No sites match “{{ search }}”.</p>

    <section v-for="group in groups" :key="group.name" class="group">
      <h2>{{ group.name }}</h2>
      <ul class="sites">
        <li v-for="site in group.sites" :key="site.id" class="site">
          <div class="site-main">
            <div class="site-name">{{ site.name }}</div>
            <div class="muted small">{{ site.login_url }}</div>
            <div class="muted small">
              {{ site.username }} · last used {{ lastUsed(site) }}
              <span v-if="site.allow_self_signed" class="tag" title="Certificate errors are ignored for this site">self-signed</span>
              <span v-if="site.window_mode === 'released'" class="tag" title="The window is handed over after login">released</span>
            </div>
          </div>
          <div class="site-actions">
            <template v-if="confirming === site.id">
              <span class="small">Delete {{ site.name }}?</span>
              <button class="danger" @click="confirmDelete(site)">Delete</button>
              <button @click="confirming = null">Cancel</button>
            </template>
            <template v-else>
              <button disabled title="Logging in arrives with the login engine (milestone M5)">Open</button>
              <button @click="emit('edit', site)">Edit</button>
              <button @click="confirming = site.id">Delete</button>
            </template>
          </div>
        </li>
      </ul>
    </section>
  </template>
</template>
