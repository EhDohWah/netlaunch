<script setup>
// The site list as a data grid: one row per site, columns that sort, a
// selected row, and (on the Groups page) a foldable header row per group.
//
// Keys, while the grid has the focus: Up/Down move the selection, Enter
// opens, F2 edits, Del deletes. Double-click opens; right-click asks the
// parent for a context menu.
import { computed, nextTick, reactive, ref } from 'vue';
import Icon from './Icon.vue';
import { lastUsed, siteStatus } from '../states.js';

const props = defineProps({
  // Already filtered by the search box and the chosen group.
  sites: { type: Array, required: true },
  logins: { type: Object, required: true },
  sessions: { type: Object, required: true },
  selectedId: { type: Number, default: null },
  // One header row per group (the Groups page).
  grouped: { type: Boolean, default: false },
});
const emit = defineEmits(['select', 'open', 'edit', 'delete', 'menu', 'group-menu']);

const COLUMNS = [
  { key: 'name', label: 'Name' },
  { key: 'group_name', label: 'Group' },
  { key: 'login_url', label: 'URL' },
  { key: 'username', label: 'Username' },
  { key: 'last_used_at', label: 'Last used' },
  { key: 'status', label: 'Status' },
];
// Grouped, the Group column would only repeat the header row.
const columns = computed(() => (props.grouped ? COLUMNS.filter((c) => c.key !== 'group_name') : COLUMNS));

// null: the vault's order (group, then name). Else { key, up }.
const sort = ref(null);
function sortBy(key) {
  if (key === 'status') return;
  if (sort.value?.key !== key) sort.value = { key, up: true };
  else if (sort.value.up) sort.value = { key, up: false };
  else sort.value = null;
}

const sorted = computed(() => {
  if (!sort.value) return props.sites;
  const { key, up } = sort.value;
  const text = (site) => String(site[key] ?? '').toLowerCase();
  return [...props.sites].sort((a, b) => text(a).localeCompare(text(b), undefined, { numeric: true }) * (up ? 1 : -1));
});

// Group names folded shut.
const folded = reactive({});

// The rows to draw, in order: { group, count } headers and { site } rows.
const rows = computed(() => {
  if (!props.grouped) return sorted.value.map((site) => ({ site }));
  const groups = new Map();
  for (const site of sorted.value) {
    const name = site.group_name || '';
    if (!groups.has(name)) groups.set(name, []);
    groups.get(name).push(site);
  }
  // Alphabetical, with ungrouped sites last.
  const names = [...groups.keys()].sort((a, b) => (a === '') - (b === '') || a.localeCompare(b));
  const result = [];
  for (const name of names) {
    result.push({ group: name, count: groups.get(name).length });
    if (!folded[name]) for (const site of groups.get(name)) result.push({ site });
  }
  return result;
});

const statusOf = (site) => siteStatus(site, props.sessions, props.logins);

const body = ref(null);
function move(step) {
  const visible = rows.value.filter((r) => r.site).map((r) => r.site);
  if (!visible.length) return;
  const at = visible.findIndex((s) => s.id === props.selectedId);
  const next = visible[Math.min(visible.length - 1, Math.max(0, at === -1 ? 0 : at + step))];
  emit('select', next.id);
  nextTick(() => body.value?.querySelector('tr.selected')?.scrollIntoView({ block: 'nearest' }));
}

function key(event) {
  const site = props.sites.find((s) => s.id === props.selectedId);
  if (event.key === 'ArrowDown') move(1);
  else if (event.key === 'ArrowUp') move(-1);
  else if (event.key === 'Enter' && site) emit('open', site);
  else if (event.key === 'F2' && site) emit('edit', site);
  else if (event.key === 'Delete' && site) emit('delete', site);
  else return;
  event.preventDefault();
}

function menu(event, site) {
  emit('select', site.id);
  emit('menu', { site, x: event.clientX, y: event.clientY });
}

defineExpose({ focus: () => body.value?.focus() });
</script>

<template>
  <div ref="body" class="grid-wrap" tabindex="0" @keydown="key">
    <table class="grid">
      <thead>
        <tr>
          <th v-for="c in columns" :key="c.key" :class="`col-${c.key}`" :aria-sort="sort?.key === c.key ? (sort.up ? 'ascending' : 'descending') : undefined">
            <button v-if="c.key !== 'status'" class="th" @click="sortBy(c.key)">
              {{ c.label }}
              <Icon v-if="sort?.key === c.key" :name="sort.up ? 'sortUp' : 'sortDown'" />
            </button>
            <span v-else class="th">{{ c.label }}</span>
          </th>
        </tr>
      </thead>
      <tbody>
        <template v-for="row in rows" :key="row.site ? row.site.id : `g:${row.group}`">
          <tr v-if="!row.site" class="group-row" @contextmenu.prevent="emit('group-menu', { group: row.group, x: $event.clientX, y: $event.clientY })">
            <td :colspan="columns.length">
              <button class="fold" :aria-expanded="!folded[row.group]" @click="folded[row.group] = !folded[row.group]">
                <Icon :name="folded[row.group] ? 'right' : 'down'" />
                {{ row.group || 'Ungrouped' }}
                <span class="count">{{ row.count }}</span>
              </button>
            </td>
          </tr>
          <tr
            v-else
            :class="{ selected: row.site.id === selectedId, indent: grouped }"
            @click="emit('select', row.site.id)"
            @dblclick="emit('open', row.site)"
            @contextmenu.prevent="menu($event, row.site)"
          >
            <td class="name">{{ row.site.name }}</td>
            <td v-if="!grouped">{{ row.site.group_name }}</td>
            <td class="dim" :title="row.site.login_url">{{ row.site.login_url }}</td>
            <td>{{ row.site.username }}</td>
            <td class="dim">{{ lastUsed(row.site) }}</td>
            <td>
              <span class="badges">
                <span v-if="statusOf(row.site)" class="badge" :class="statusOf(row.site).kind" :title="statusOf(row.site).message">{{ statusOf(row.site).label }}</span>
                <span v-if="row.site.allow_self_signed" class="badge" title="Certificate errors are ignored for this site">self-signed</span>
                <span v-if="!row.site.username" class="badge" title="This device asks for no username">password only</span>
                <span v-if="row.site.window_mode === 'released'" class="badge" title="The window is handed over after login">released</span>
              </span>
            </td>
          </tr>
        </template>
      </tbody>
    </table>
    <p v-if="!rows.length" class="grid-empty">No sites match.</p>
  </div>
</template>
