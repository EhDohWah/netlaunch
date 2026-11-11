import { createApp } from 'vue';
import App from './App.vue';
import './style.css';
import { initTheme } from './theme.js';

initTheme();

createApp(App).mount('#app');
