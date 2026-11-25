<script setup>
// The left pane's list of sites: two lines per site (name, then address
// and username) with its login status on the right, under a foldable
// heading per group.
//
// To the keyboard and to screen readers it is one "tree": Tab reaches it
// once, then
//   Up / Down / Home / End   move through the rows,
//   Right / Left             unfold or fold a group (Left on a site goes to its group),
//   Enter                    opens the site (or folds/unfolds a group),
//   F2 / Del                 edit / delete a site; rename / remove a group,
//   Shift+F10 or the Menu key  the right-click menu,
//   typing letters           jumps to the next site whose name starts with them.
// The rows are plain elements with tree roles rather than buttons: that
// is the standard pattern for a list driven from its container.
import { computed, nextTick, reactive, ref, watch } from 'vue';
import Icon from './Icon.vue';
import { hostOf, siteStatus } from '../states.js';

const props = defineProps({
  // Already narrowed by the search box and the All / Open now choice.
  sites: { type: Array, required: true },
  logins: { type: Object, required: true },
  sessions: { type: Object, required: true },
  selectedId: { type: Number, default: null },
  // A heading per group. Off for search results and "Open now".
  grouped: { type: Boolean, default: true },
  // Shown when there are no rows.
  emptyText: { type: String, default: 'No sites.' },
});
const emit = defineEmits(['select', 'open', 'edit', 'delete', 'menu', 'group-menu', 'rename-group', 'remove-group']);

// Group names folded shut, remembered between runs of the app.
const FOLDED_KEY = 'netlaunch-folded-groups';
const folded = reactive(readFolded());
function readFolded() {
  try {
    const names = JSON.parse(localStorage.getItem(FOLDED_KEY) ?? '[]');
    return Object.fromEntries(names.map((name) => [name, true]));
  } catch {
    return {};
  }
}
watch(folded, (now) => {
  try {
    localStorage.setItem(FOLDED_KEY, JSON.stringify(Object.keys(now).filter((name) => now[name])));
  } catch {
    // Not saved; it applies until the app closes.
  }
});

// The rows to draw, in order: { group } headings and { site } rows. `key`
// names a row in this component; `dom` is its element id.
const rows = computed(() => {
  if (!props.grouped) return props.sites.map((site) => ({ key: `s:${site.id}`, dom: `nl-site-${site.id}`, site }));
  // Keyed without regard to capitals, so sites saved as "switches" and
  // "Switches" (possible in older data) share one heading, shown in the
  // spelling met first.
  const groups = new Map();
  const spelling = new Map();
  for (const site of props.sites) {
    const name = site.group_name || '';
    const key = name.toLowerCase();
    if (!spelling.has(key)) spelling.set(key, name);
    const shown = spelling.get(key);
    if (!groups.has(shown)) groups.set(shown, []);
    groups.get(shown).push(site);
  }
  // Alphabetical, with the sites that have no group last.
  const names = [...groups.keys()].sort((a, b) => (a === '') - (b === '') || a.localeCompare(b));
  const result = [];
  names.forEach((name, i) => {
    result.push({ key: `g:${name}`, dom: `nl-group-${i}`, group: name, count: groups.get(name).length });
    if (!folded[name]) for (const site of groups.get(name)) result.push({ key: `s:${site.id}`, dom: `nl-site-${site.id}`, site });
  });
  return result;
});

// The row the keyboard is on: a site (which is then also the selected
// one) or a group heading.
const activeKey = ref(null);
watch(
  () => props.selectedId,
  (id) => {
    if (id !== null) activeKey.value = `s:${id}`;
  },
  { immediate: true },
);
const active = computed(() => rows.value.find((r) => r.key === activeKey.value) ?? null);

const root = ref(null);
function activate(row) {
  activeKey.value = row.key;
  if (row.site) emit('select', row.site.id);
  nextTick(() => document.getElementById(row.dom)?.scrollIntoView({ block: 'nearest' }));
}

function move(to) {
  const list = rows.value;
  if (!list.length) return;
  const at = list.findIndex((r) => r.key === activeKey.value);
  const index = to === 'first' ? 0 : to === 'last' ? list.length - 1 : Math.min(list.length - 1, Math.max(0, at === -1 ? 0 : at + to));
  activate(list[index]);
}

/** The heading row a site sits under. */
function headingOf(site) {
  const name = (site.group_name || '').toLowerCase();
  return rows.value.find((r) => !r.site && r.group.toLowerCase() === name);
}

function fold(name, shut) {
  folded[name] = shut;
  // The row the keyboard was on may just have been hidden.
  if (shut && !rows.value.some((r) => r.key === activeKey.value)) activeKey.value = `g:${name}`;
}

// Letters typed in quick succession, for jumping to a site by name.
let typed = '';
let typedAt = 0;
function typeAhead(char) {
  const now = Date.now();
  typed = now - typedAt < 700 ? typed + char : char;
  typedAt = now;
  const sites = rows.value.filter((r) => r.site);
  const at = sites.findIndex((r) => r.key === activeKey.value);
  // Start after the current row for a single letter, so repeating it
  // walks through every match.
  const order = [...sites.slice(at + (typed.length === 1 ? 1 : 0)), ...sites.slice(0, at + 1)];
  const hit = order.find((r) => r.site.name.toLowerCase().startsWith(typed.toLowerCase()));
  if (hit) activate(hit);
}

function openMenu(row, x, y) {
  if (row.site) emit('menu', { site: row.site, x, y });
  else emit('group-menu', { group: row.group, x, y });
}

function key(event) {
  const row = active.value;
  const site = row?.site ?? null;
  const k = event.key;
  if (k === 'ArrowDown') move(1);
  else if (k === 'ArrowUp') move(-1);
  else if (k === 'Home') move('first');
  else if (k === 'End') move('last');
  else if (k === 'ArrowRight' && row && !site) folded[row.group] ? fold(row.group, false) : move(1);
  else if (k === 'ArrowLeft' && row && !site) fold(row.group, true);
  else if (k === 'ArrowLeft' && site && props.grouped) activate(headingOf(site));
  else if (k === 'Enter' && site) emit('open', site);
  else if ((k === 'Enter' || k === ' ') && row && !site) fold(row.group, !folded[row.group]);
  else if (k === 'F2' && site) emit('edit', site);
  else if (k === 'Delete' && site) emit('delete', site);
  // On a group heading the same keys rename and remove the group. The
  // "No group" heading isn't a group, so they do nothing there.
  else if (k === 'F2' && row?.group) emit('rename-group', row.group);
  else if (k === 'Delete' && row?.group) emit('remove-group', row.group);
  else if ((k === 'ContextMenu' || (k === 'F10' && event.shiftKey)) && row) {
    const box = document.getElementById(row.dom).getBoundingClientRect();
    openMenu(row, box.left + 24, box.bottom - 4);
  } else if (k.length === 1 && k !== ' ' && !event.ctrlKey && !event.altKey && !event.metaKey) typeAhead(k);
  else return;
  event.preventDefault();
}

// Tabbing in with nothing active yet: start on the first row.
function focused() {
  if (!active.value && rows.value.length) activeKey.value = rows.value[0].key;
}

const statusOf = (site) => siteStatus(site, props.sessions, props.logins);

defineExpose({
  focus: () => root.value?.focus(),
  // A renamed group stays folded (or open) as it was.
  groupRenamed(from, to) {
    if (folded[from]) folded[to] = true;
    delete folded[from];
    if (activeKey.value === `g:${from}`) activeKey.value = `g:${to}`;
  },
});
</script>

<template>
  <div ref="root" class="sitelist" role="tree" aria-label="Sites" tabindex="0" :aria-activedescendant="active?.dom" @keydown="key" @focus="focused">
    <template v-for="row in rows" :key="row.key">
      <div
        v-if="!row.site"
        :id="row.dom"
        class="group-head"
        :class="{ active: row.key === activeKey }"
        role="treeitem"
        aria-level="1"
        :aria-expanded="!folded[row.group]"
        aria-selected="false"
        @click="((activeKey = row.key), fold(row.group, !folded[row.group]))"
        @contextmenu.prevent="((activeKey = row.key), openMenu(row, $event.clientX, $event.clientY))"
      >
        <Icon :name="folded[row.group] ? 'right' : 'down'" />
        <span class="grow">{{ row.group || 'No group' }}</span>
        <span class="count">{{ row.count }}</span>
      </div>
      <div
        v-else
        :id="row.dom"
        class="item"
        :class="{ selected: row.site.id === selectedId, active: row.key === activeKey }"
        role="treeitem"
        :aria-level="grouped ? 2 : 1"
        :aria-selected="row.site.id === selectedId"
        @click="activate(row)"
        @dblclick="emit('open', row.site)"
        @contextmenu.prevent="(activate(row), openMenu(row, $event.clientX, $event.clientY))"
      >
        <span class="item-text">
          <span class="item-name">{{ row.site.name }}</span>
          <span class="item-sub">{{ hostOf(row.site) }} · {{ row.site.username || 'password only' }}</span>
        </span>
        <span v-if="statusOf(row.site)" class="state" :class="statusOf(row.site).kind" :title="statusOf(row.site).message">
          <span class="dot"></span>{{ statusOf(row.site).label }}
        </span>
      </div>
    </template>
    <p v-if="!rows.length" class="list-empty">{{ emptyText }}</p>
  </div>
</template>
