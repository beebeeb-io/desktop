import jsxA11y from 'eslint-plugin-jsx-a11y'
import reactHooks from 'eslint-plugin-react-hooks'
import tseslint from 'typescript-eslint'

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
    },
  },
]
