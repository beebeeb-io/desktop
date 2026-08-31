import jsxA11y from 'eslint-plugin-jsx-a11y'
import reactHooks from 'eslint-plugin-react-hooks'
import tseslint from 'typescript-eslint'
import noAdHocErrorSurface from './eslint-rules/no-ad-hoc-error-surface.mjs'

const jsxA11yRecommended = jsxA11y.flatConfigs.recommended

export default [
  {
    ignores: [
      'dist/**',
      'node_modules/**',
      'src-tauri/target/**',
    ],
  },
  {
    files: ['src/**/*.{jsx,tsx}'],
    plugins: {
      ...jsxA11yRecommended.plugins,
      'react-hooks': reactHooks,
      beebeeb: { rules: { 'no-ad-hoc-error-surface': noAdHocErrorSurface } },
    },
    languageOptions: {
      ...jsxA11yRecommended.languageOptions,
      parser: tseslint.parser,
      parserOptions: {
        ecmaFeatures: { jsx: true },
        ecmaVersion: 'latest',
        sourceType: 'module',
      },
    },
    rules: {
      ...jsxA11yRecommended.rules,
      'react-hooks/exhaustive-deps': 'error',
      'react-hooks/rules-of-hooks': 'error',
      'beebeeb/no-ad-hoc-error-surface': 'error',
    },
  },
]
