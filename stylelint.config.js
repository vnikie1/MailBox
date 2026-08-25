/**
 * Standing rule 1 (PROMPT.md): a component may never hardcode a colour, size, radius,
 * duration or easing value. Only the three token files may contain raw values.
 *
 * That rule is unenforceable by review alone, so it is enforced here: any `*.module.css`
 * containing a hex colour, an rgb()/hsl() literal, a px length, a time value or a raw
 * easing curve fails the build. `src/styles/tokens/*.css` and `global.css` are exempt by
 * virtue of not being CSS Modules.
 *
 * Note these are real RegExp objects, not stylelint's `'/pattern/'` string form — the
 * string form needs every backslash doubled and is a reliable source of silent misfires.
 */

const RAW_VALUE_PATTERNS = [
  // hex colours: #fff, #ffffff, #ffffffff
  /#[0-9a-fA-F]{3,8}(\b|$)/,
  // colour literals that should be a semantic token
  /\b(rgba?|hsla?)\s*\(/,
  // px lengths — a bare `0` is fine and stays allowed
  /(^|[^\w.-])\d*\.?\d+px\b/,
  // durations
  /(^|[^\w.-])\d*\.?\d+m?s\b/,
  // easing curves
  /cubic-bezier\s*\(/,
  /(^|[\s,(])linear\s*\([\d.]/,
]

export default {
  extends: ['stylelint-config-standard'],
  rules: {
    // CSS Modules vocabulary
    'selector-pseudo-class-no-unknown': [true, { ignorePseudoClasses: ['global', 'local'] }],
    'property-no-unknown': [true, { ignoreProperties: ['composes'] }],
    'custom-property-pattern': null,
    'selector-class-pattern': '^[a-z][a-zA-Z0-9]+$',
    // Stylelint's preference for shorthands fights the token system more than it helps.
    'declaration-block-no-redundant-longhand-properties': null,
    'alpha-value-notation': null,
    'color-function-notation': null,

    // Six-digit hex throughout, matching docs/02 §3 literally: the token files are
    // audited against that table, and #fff vs #ffffff drift makes that harder.
    'color-hex-length': null,
    // Bare-string @import is valid CSS and is what Vite and the docs both use.
    'import-notation': null,
    // Blank lines group the tokens by role, which is the whole readability story
    // of these files.
    'custom-property-empty-line-before': null,
  },
  overrides: [
    {
      files: ['**/*.module.css'],
      rules: {
        'declaration-property-value-disallowed-list': [
          { '/.*/': RAW_VALUE_PATTERNS },
          {
            message:
              'Hardcoded value. PROMPT.md standing rule 1: add a token to src/styles/tokens/ and reference it with var().',
          },
        ],
      },
    },
  ],
  ignoreFiles: ['dist/**', 'node_modules/**', 'src-tauri/**', 'coverage/**'],
}
