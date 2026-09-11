import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App } from './App';
import { applyTheme, themeStore } from './stores/themeStore';
import './styles.css';

// Stamp the stored theme before the first paint so there is no flash of the wrong palette.
applyTheme(themeStore.getState().preference);

const host = document.getElementById('root');
if (!host) throw new Error('#root is missing from index.html');

createRoot(host).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
