import '@hoi/tokens/fonts.css';
import '@hoi/tokens/tokens.css';
import './app.css';
import { mount } from 'svelte';
import App from './App.svelte';
import { prepaintTheme } from './lib/app.svelte';
import { registerServiceWorker } from './lib/sw.svelte';

prepaintTheme();

function boot() {
  const target = document.getElementById('app');
  if (!target) throw new Error('#app missing');
  mount(App, { target });
  registerServiceWorker();
}

boot();
