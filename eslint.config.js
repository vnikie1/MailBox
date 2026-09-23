import js from '@eslint/js'
import globals from 'globals'
import tseslint from 'typescript-eslint'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'

export default tseslint.config(
  {
    ignores: [
      'dist',
      'src-tauri/target',
      'src-tauri/gen',
      'coverage',
      'playwright-report',
      'test-results',
      // Written by `cargo test` from the Rust types; formatting them here would be undone
      // on the next run and the diff would be pure noise.
      'src/lib/generated',
      // The designer's raw icon export, untracked (see .gitignore). Its scripts are the design
      // tool's, not ours, and ESLint does not read .gitignore.
      'Halcyon Mail App Logo',
    ],
  },

  js.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  ...tseslint.configs.stylisticTypeChecked,

  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      'react-refresh/only-export-components': ['warn', { allowConstantExport: true }],
      '@typescript-eslint/consistent-type-imports': [
        'error',
        { prefer: 'type-imports', fixStyle: 'inline-type-imports' },
      ],
      // The IPC boundary is the one place unknown-shaped data enters the UI. It is
      // validated there and nowhere else, so banning `any` everywhere else is right.
      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/restrict-template-expressions': ['error', { allowNumber: true }],
    },
  },

  // One popup button in the app, not eight.
  //
  // Windows draws an open <select> itself, from the control's own background, so every
  // hand-styled one is a chance to open a list with no surface. Eight of them did — the compose
  // window's From picker was reported on 2026-09-17 with every account but the hovered one
  // unreadable, and the rules editors and the account assistant had the same fault — while
  // `ui/Select` had been fixed for it weeks earlier. Nothing stopped a feature writing its own.
  {
    files: ['src/**/*.tsx'],
    ignores: ['src/ui/Select.tsx'],
    rules: {
      'no-restricted-syntax': [
        'error',
        {
          selector: "JSXOpeningElement[name.name='select']",
          message:
            'Use the Select primitive from @/ui. Windows draws the open list from the control, and a hand-styled <select> opens one with no surface in the dark theme.',
        },
      ],
    },
  },

  // @floating-ui/react declares refs.setReference and refs.setFloating as methods, so
  // unbound-method fires every time one is passed to a ref prop — which is the library’s
  // documented and only usage. Wrapping them in arrow functions to satisfy the rule would
  // create a new ref callback on every render, detaching and reattaching the node each
  // time. Scoped to the primitives that actually host a floating layer.
  {
    files: ['src/ui/{Popover,Tooltip,Menu,ContextMenu,Sheet}.tsx'],
    rules: { '@typescript-eslint/unbound-method': 'off' },
  },

  // Node-side config files.
  {
    files: ['**/*.{ts,tsx}', 'vite.config.ts', 'vitest.config.ts', 'playwright.config.ts'],
    languageOptions: { globals: { ...globals.browser, ...globals.node } },
  },

  // The tool configs are plain JS and are not part of any tsconfig, so the type-aware
  // rules have nothing to work from. Turning them off here is the supported way to say
  // so, rather than dragging .js files into the TypeScript program.
  {
    files: ['**/*.{js,mjs,cjs}'],
    ...tseslint.configs.disableTypeChecked,
    languageOptions: {
      globals: { ...globals.node },
      parserOptions: { projectService: false },
    },
  },

  // CommonJS dev tooling. require() is the correct module system in a .cjs script, not
  // a lapse — this must come last so it wins over the block above.
  //
  // `document` and `window` are declared because these scripts drive a browser: the callback
  // passed to Playwright's `page.evaluate` is serialised and run *in the page*, so it is
  // browser code that happens to be written inside a Node file. Without this, every DOM read
  // in a driver script is a no-undef error.
  {
    files: ['tools/**/*.cjs'],
    languageOptions: {
      globals: { document: 'readonly', window: 'readonly', getComputedStyle: 'readonly' },
    },
    rules: { '@typescript-eslint/no-require-imports': 'off' },
  },
)
