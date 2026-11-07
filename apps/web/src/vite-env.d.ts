/// <reference types="svelte" />
/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** 'mock' selects the in-memory dev backend. */
  readonly VITE_HOI_BACKEND?: string;
  /** Shared HTTPS registry, or /registry for the bundled web deployment; empty disables registration. */
  readonly VITE_HOI_REGISTRY_URL?: string;
}
interface ImportMeta {
  readonly env: ImportMetaEnv;
}

/** Set at build time: true when building the UI for the Tauri desktop shell. */
declare const __HOI_DESKTOP__: boolean;
/** Set at build time: true for the node setup app's window (apps/setup). */
declare const __HOI_SETUP__: boolean;
/** Explicit encrypted relay endpoints, with no decryption keys or credentials. */
declare const __HOI_RELAY_URLS__: string[];
/** Local sandbox build only: isolated test identities and local control integration. */
declare const __HOI_SANDBOX__: boolean;
/** App version from package.json, set at build time. */
declare const __HOI_VERSION__: string;
declare const __HOI_PUBLICATION__: import('../../../scripts/publication.mjs').Publication;
/** Set by release.json for both browser and desktop builds. */
declare const __HOI_PREVIEW__: boolean;
/** Informational build snapshot. Runtime notices cannot alter compiled protocol support. */
declare const __HOI_RELEASE__: import('../../../scripts/release-schema.mjs').ReleaseConfig;
/** Protocol version shown in the preview banner. */
declare const __HOI_PROTOCOL__: string;
/** Who publishes this build (HOI_PUBLISHER) and how to reach them (HOI_CONTACT), for the privacy notice. */
declare const __HOI_PUBLISHER__: string;
declare const __HOI_CONTACT__: string;
