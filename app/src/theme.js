// Light or dark mode. The colours are the CSS variables in style.css;
// this only picks which set applies, by putting `data-theme` on <html>.
//
// The choice is kept in localStorage. Until the admin picks one, the app
// follows the Windows setting.
import { ref } from 'vue';
import { setWindowTheme } from './api.js';

const KEY = 'netlaunch-theme';

function stored() {
  try {
    const value = localStorage.getItem(KEY);
    return value === 'light' || value === 'dark' ? value : null;
  } catch {
    // Storage can be unavailable; the theme still works for this run.
    return null;
  }
}

function system() {
  return window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

/** 'light' or 'dark': what the app shows now. */
export const theme = ref(stored() ?? system());

function apply() {
  document.documentElement.dataset.theme = theme.value;
  // The title bar follows too. Not being able to (a plain browser, in
  // development) doesn't matter.
  setWindowTheme(theme.value === 'dark').catch(() => {});
}

/** Called once, before the app is drawn, so it never flashes the other theme. */
export function initTheme() {
  apply();
}

export function toggleTheme() {
  theme.value = theme.value === 'dark' ? 'light' : 'dark';
  apply();
  try {
    localStorage.setItem(KEY, theme.value);
  } catch {
    // Not saved; it applies until the app closes.
  }
}
