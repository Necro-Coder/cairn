/**
 * The vocabulary of the boundary between the interface and the core, and the shape any
 * implementation of that boundary has to have.
 *
 * This file imports nothing. It describes the frontier without being on either side of
 * it, which is what lets two different modules be checked against the same contract: the
 * one that calls the Rust core, and the one that stands in for it while the interface is
 * being looked at in an ordinary browser.
 *
 * The point of `IpcSurface` is that the two cannot drift apart. Add a function to one and
 * not the other and the build stops, rather than the stand-in quietly falling behind until
 * somebody designs a screen against a boundary that no longer matches the real one.
 *
 * TypeScript types do not exist at runtime, so nothing here is a guarantee about a value.
 * What arrives from the core is trusted because it comes from our own core, not because it
 * is annotated. Anything that is not our own core is hostile input and gets validated.
 */

/**
 * Which build produced the running binary.
 *
 * @public part of the boundary vocabulary, exported whether or not anything imports it
 * by name today.
 */
export type BuildProfile = 'debug' | 'release';

/** Name, version and build profile of the running application. */
export interface AppInfo {
  readonly name: string;
  readonly version: string;
  readonly profile: BuildProfile;
}

/**
 * Whether the encrypted database has been opened.
 *
 * Only one value exists until storage is implemented.
 *
 * @public part of the boundary vocabulary, exported whether or not anything imports it
 * by name today.
 */
export type DatabaseStatus = 'notInitialized';

/**
 * What the application will admit to about itself.
 *
 * Deliberately contains nothing that identifies the person or the machine: no paths, no
 * user name, no host name. A screenshot of the diagnostics screen is safe to share.
 */
export interface Diagnostics {
  readonly app: AppInfo;
  readonly os: string;
  readonly arch: string;
  readonly webviewVersion: string | null;
  readonly database: DatabaseStatus;
  readonly uptimeMs: number;
}

/**
 * Everything the interface may ask of whatever is on the other side of the boundary.
 *
 * Every implementation is checked against this, so a function that exists on one side and
 * not the other is a compile error rather than a surprise at runtime. That is the whole
 * reason the type exists; a written convention would survive exactly until the first time
 * somebody was in a hurry.
 */
export interface IpcSurface {
  /**
   * What to put on screen to say that none of this is real, or `null` when it is.
   *
   * It belongs to the implementation rather than to the interface because the module that
   * makes the data up is the one that knows it did. The real boundary answers `null`, so
   * the warning text does not exist anywhere in a production build: it is not hidden by a
   * condition, it is simply not there.
   */
  readonly previewNotice: string | null;

  /** Reads the name, version and build profile of the running application. */
  readonly fetchAppInfo: () => Promise<AppInfo>;

  /** Reads a snapshot of the application state for the diagnostics screen. */
  readonly fetchDiagnostics: () => Promise<Diagnostics>;
}
