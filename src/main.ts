import { mount } from 'svelte';

import App from './App.svelte';
import './lib/styles/tokens.css';
import './lib/styles/base.css';

const target = document.getElementById('app');

if (target === null) {
  // index.html is ours and ships inside the binary, so a missing mount point is a build
  // that was assembled wrongly rather than anything a user can cause. Failing here is
  // better than mounting nothing and showing a blank window with no explanation.
  throw new Error('No se ha encontrado el elemento #app en el que montar la aplicación.');
}

export default mount(App, { target });
