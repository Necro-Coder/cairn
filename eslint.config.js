import js from '@eslint/js';
import globals from 'globals';
import svelte from 'eslint-plugin-svelte';
import svelteParser from 'svelte-eslint-parser';
import tseslint from 'typescript-eslint';

import svelteConfig from './svelte.config.js';

/**
 * Lint configuration.
 *
 * Most of this is ordinary hygiene. Three rules are not, and they are the reason the
 * linter is a blocking gate rather than a suggestion:
 *
 * `svelte/no-at-html-tags` stops `{@html}` from ever appearing. A vault entry imported
 * from a file someone else wrote is hostile input. Rendering it as HTML turns a malicious
 * backup into script running inside the WebView, and script inside the WebView is one
 * step from the command boundary. There is no legitimate use for it in this project.
 *
 * `no-restricted-imports` keeps `@tauri-apps/api` out of every file except `src/lib/ipc.ts`.
 * The value of a single narrow boundary is entirely lost if a component can reach past it,
 * and nobody notices that happening in review.
 *
 * `no-restricted-globals` and `no-restricted-syntax` ban `eval`, `new Function` and
 * `innerHTML`, which are the other three ways to execute or inject markup at runtime.
 */
export default tseslint.config(
  {
    ignores: [
      'dist/**',
      'dist-preview/**',
      'node_modules/**',
      'target/**',
      'src-tauri/**',
      'crates/**',
    ],
  },

  js.configs.recommended,
  ...tseslint.configs.recommendedTypeChecked,
  ...svelte.configs.recommended,

  {
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: 'module',
      globals: { ...globals.browser },
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
        extraFileExtensions: ['.svelte'],
      },
    },
  },

  {
    // A `.svelte.ts` file is a module that may use runes, so it needs the same parser as a
    // component. Without this the first rune in one is reported as a syntax error.
    files: ['**/*.svelte', '**/*.svelte.ts'],
    languageOptions: {
      parser: svelteParser,
      parserOptions: {
        parser: tseslint.parser,
        projectService: true,
        extraFileExtensions: ['.svelte'],
        // Without the compiler options the parser cannot resolve runes, and every typed
        // rule then reports `any` for values that are in fact fully typed.
        svelteConfig,
      },
    },
  },

  {
    rules: {
      // The defence against a malicious import becoming script execution.
      'svelte/no-at-html-tags': 'error',
      'svelte/no-target-blank': 'error',

      'no-restricted-globals': [
        'error',
        { name: 'eval', message: 'Executing a string at runtime is never needed here.' },
      ],
      'no-restricted-syntax': [
        'error',
        {
          selector: 'NewExpression[callee.name="Function"]',
          message: 'new Function is eval with extra steps.',
        },
        {
          selector: "MemberExpression[property.name='innerHTML']",
          message:
            'Use textContent. If markup is genuinely required, it has to be sanitised first.',
        },
        {
          selector: "MemberExpression[property.name='outerHTML']",
          message:
            'Use textContent. If markup is genuinely required, it has to be sanitised first.',
        },
      ],

      // Everything that talks to the Rust core goes through one auditable file.
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              group: ['@tauri-apps/api', '@tauri-apps/api/*'],
              message:
                'Only src/lib/ipc.ts may import the Tauri API. Add a typed function there and call that instead.',
            },
          ],
        },
      ],

      eqeqeq: ['error', 'always'],
      'no-console': 'error',
      'no-debugger': 'error',
      'prefer-const': 'error',
      'no-var': 'error',

      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/no-unsafe-assignment': 'error',
      '@typescript-eslint/no-unsafe-member-access': 'error',
      '@typescript-eslint/no-unsafe-call': 'error',
      '@typescript-eslint/no-unsafe-return': 'error',
      '@typescript-eslint/no-floating-promises': 'error',
      '@typescript-eslint/no-misused-promises': 'error',
      '@typescript-eslint/consistent-type-imports': [
        'error',
        { prefer: 'type-imports', fixStyle: 'inline-type-imports' },
      ],
      '@typescript-eslint/explicit-function-return-type': [
        'error',
        { allowExpressions: true, allowTypedFunctionExpressions: true },
      ],
    },
  },

  {
    // The one file allowed through the boundary rule, because it is the boundary.
    files: ['src/lib/ipc.ts'],
    rules: {
      'no-restricted-imports': 'off',
    },
  },

  // The two blocks below switch off the rules that need type information, so they have to
  // come last: in a flat configuration the final entry that matches a file wins.

  {
    // The isolation application is a plain browser script that lives outside the frontend
    // build on purpose. It is still linted, but there is no TypeScript project for it to
    // be type-checked against, and giving it one would mean bundling it with the code it
    // is supposed to be isolated from.
    files: ['src-isolation/**/*.js'],
    languageOptions: {
      globals: { ...globals.browser },
    },
    ...tseslint.configs.disableTypeChecked,
  },

  {
    // Type-aware rules are switched off inside components, and this is a deliberate
    // division of labour rather than a gap.
    //
    // The parser compiles a component into virtual TypeScript in order to type it, and
    // that transformation does not carry the narrowing a template performs. A block
    // guarded by `{#if status.kind === 'ready'}` reads as `any` to these rules, so they
    // report a union that is in fact fully discriminated. Leaving them on would mean
    // either a wall of false positives or a scattering of suppressions, and a suppression
    // people learn to add without reading is worse than no rule.
    //
    // Types in components are checked by `npm run check`, which runs the Svelte compiler
    // itself and understands the narrowing. That gate is not optional, and it is the one
    // that would catch a genuine type error here.
    //
    // Everything above that defends the WebView is syntactic and still applies to every
    // component: no `{@html}`, no `eval`, no `innerHTML`, and no reaching around
    // `src/lib/ipc.ts`.
    files: ['**/*.svelte'],
    ...tseslint.configs.disableTypeChecked,
    rules: {
      ...tseslint.configs.disableTypeChecked.rules,
      'svelte/no-at-html-tags': 'error',
      'svelte/no-target-blank': 'error',
    },
  },

  {
    // Configuration files run in Node and are not part of the application.
    files: ['*.config.js', '*.config.ts', 'eslint.config.js', 'svelte.config.js'],
    languageOptions: {
      globals: { ...globals.node },
    },
    ...tseslint.configs.disableTypeChecked,
  },

  {
    // The design token gate and its tests. They run in Node, outside the frontend build,
    // and there is no TypeScript project for them to be checked against: they are plain
    // modules deliberately kept free of anything that would need one.
    files: ['scripts/**/*.mjs'],
    languageOptions: {
      globals: { ...globals.node },
    },
    ...tseslint.configs.disableTypeChecked,
    rules: {
      ...tseslint.configs.disableTypeChecked.rules,
      // JavaScript has nowhere to write a return type. What these functions return is
      // documented in JSDoc above each one, which is where a reader of a `.mjs` file
      // looks anyway.
      '@typescript-eslint/explicit-function-return-type': 'off',
    },
  },
);
