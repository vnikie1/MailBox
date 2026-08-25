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
  {
    files: ['tools/**/*.cjs'],
    rules: { '@typescript-eslint/no-require-imports': 'off' },
  },
)
