<script setup>
// Fluent System Icons (Microsoft, MIT licence), 16 px "regular" set. Each
// SVG is pulled into the bundle as text (`?raw`), so only the icons named
// here ship with the app. They have no colour of their own: CSS fills
// them with the text colour.
import add from '@fluentui/svg-icons/icons/add_16_regular.svg?raw';
import open from '@fluentui/svg-icons/icons/open_16_regular.svg?raw';
import edit from '@fluentui/svg-icons/icons/edit_16_regular.svg?raw';
import del from '@fluentui/svg-icons/icons/delete_16_regular.svg?raw';
import search from '@fluentui/svg-icons/icons/search_16_regular.svg?raw';
import home from '@fluentui/svg-icons/icons/home_16_regular.svg?raw';
import folder from '@fluentui/svg-icons/icons/folder_16_regular.svg?raw';
import settings from '@fluentui/svg-icons/icons/settings_16_regular.svg?raw';
import menu from '@fluentui/svg-icons/icons/navigation_16_regular.svg?raw';
import down from '@fluentui/svg-icons/icons/chevron_down_16_regular.svg?raw';
import right from '@fluentui/svg-icons/icons/chevron_right_16_regular.svg?raw';
import moon from '@fluentui/svg-icons/icons/weather_moon_16_regular.svg?raw';
import sun from '@fluentui/svg-icons/icons/weather_sunny_16_regular.svg?raw';
import eye from '@fluentui/svg-icons/icons/eye_16_regular.svg?raw';
import eyeOff from '@fluentui/svg-icons/icons/eye_off_16_regular.svg?raw';
import dismiss from '@fluentui/svg-icons/icons/dismiss_16_regular.svg?raw';
import retry from '@fluentui/svg-icons/icons/arrow_sync_16_regular.svg?raw';
import windowIcon from '@fluentui/svg-icons/icons/window_16_regular.svg?raw';
import copy from '@fluentui/svg-icons/icons/copy_16_regular.svg?raw';
import rename from '@fluentui/svg-icons/icons/rename_16_regular.svg?raw';
import release from '@fluentui/svg-icons/icons/arrow_exit_16_regular.svg?raw';
import move from '@fluentui/svg-icons/icons/folder_arrow_right_16_regular.svg?raw';
import sortUp from '@fluentui/svg-icons/icons/arrow_sort_up_16_regular.svg?raw';
import sortDown from '@fluentui/svg-icons/icons/arrow_sort_down_16_regular.svg?raw';

const ICONS = {
  add, open, edit, delete: del, search, home, folder, settings, menu, down, right, moon, sun, eye, eyeOff,
  dismiss, retry, window: windowIcon, copy, rename, release, move, sortUp, sortDown,
};

defineProps({ name: { type: String, required: true } });
</script>

<template>
  <!-- The SVG text comes from the npm package at build time, never from user data. -->
  <span class="icon" aria-hidden="true" v-html="ICONS[name]"></span>
</template>
