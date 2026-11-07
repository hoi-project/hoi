// The node setup app's window (apps/setup): the same tokens and styles as the hoi app, one screen.
import '@hoi/tokens/fonts.css';
import '@hoi/tokens/tokens.css';
import '../src/app.css';
import { mount } from 'svelte';
import Setup from '../src/setup/Setup.svelte';

const target = document.getElementById('app');
if (!target) throw new Error('#app missing');
mount(Setup, { target });
