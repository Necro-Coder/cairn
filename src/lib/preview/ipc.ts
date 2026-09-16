/**
 * A stand-in for the core, so that the interface can be opened in an ordinary browser.
 *
 * There is no Rust behind a browser tab. Without this module the two screens would show
 * nothing but an error, which makes the interface impossible to look at anywhere except
 * inside the desktop window, and in particular impossible to look at on a phone before
 * there is an application to install on one.
 *
 * What this module is not, and what nothing here can be used to claim:
 *
 * - It validates nothing about the core. There is no Rust, no encryption and no database.
 *   A screen that works here can still be broken in the real application.
 * - The numbers it returns are invented. Reading a performance budget off a preview is
 *   reading a number somebody typed into this file.
 * - It is not a test double. Tests run against the real boundary or against Rust.
 *
 * Every value is fixed and deterministic. Random data, even seeded, hides intermittent
 * mistakes and means two screenshots of the same screen cannot be compared.
 *
 * This file is never in a production build. The `$ipc` alias only points here when Vite
 * runs in preview mode, so it is not behind a condition at runtime, it is absent from the
 * module graph entirely. A gate in the pipeline searches the built bundle for the marker
 * below and fails if it finds it, because a build flag on its own is not a defence: one
 * badly placed import undoes it without saying anything.
 */

import type { AppInfo, Diagnostics, IpcSurface } from '../ipc.types';

/**
 * The marker, shown to the person and searched for in the built bundle.
 *
 * One string with two jobs on purpose. Shown on screen it is unmistakable in a way that
 * a tasteful grey label is not, and a machine token in the middle of an interface is
 * exactly the kind of thing nobody mistakes for the real application. Searched for in
 * `dist/`, it is proof about the artefact rather than about the intention behind it.
 *
 * Because it is one string used in both places, the banner cannot say one thing while the
 * gate looks for another.
 */
const MARKER = 'CAIRN-PREVIEW-MOCK-DATA';

/** Obviously invented, and obviously not a version anybody released. */
const PREVIEW_APP_INFO: AppInfo = {
  name: 'Cairn (datos de mentira)',
  version: '0.0.0-ejemplo',
  profile: 'debug',
};

/** Obviously invented. No real system reports an architecture called this. */
const PREVIEW_DIAGNOSTICS: Diagnostics = {
  app: PREVIEW_APP_INFO,
  os: 'sistema de ejemplo',
  arch: 'arquitectura de ejemplo',
  webviewVersion: 'navegador de ejemplo',
  database: 'notInitialized',
  uptimeMs: 1234,
};

/**
 * The stand-in boundary.
 *
 * Annotated with the same shared type as the real one, so the two cannot drift: a command
 * added on one side and not the other stops the build rather than waiting to be noticed.
 */
export const ipc: IpcSurface = {
  previewNotice: `Previsualización de la interfaz. Todos los datos son inventados y no hay núcleo detrás. ${MARKER}`,

  fetchAppInfo: () => Promise.resolve(PREVIEW_APP_INFO),

  fetchDiagnostics: () => Promise.resolve(PREVIEW_DIAGNOSTICS),
};
