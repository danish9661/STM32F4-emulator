import js from '@eslint/js';
import globals from 'globals';

export default [
  {
    ignores: ['site/vendor/**', '**/stm32-periph-wasm/pkg/**', '**/stm32-periph-wasm/package/**', '**/stm32-periph-wasm/target/**', '**/node_modules/**', 'website/**', '**/openhw-local-gateway/vendor/**', '**/*.wasm', '**/unicorn*.js'],
  },
  js.configs.recommended,
  {
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: 'module',
      globals: { ...globals.browser, ...globals.node },
    },
    rules: {
      'no-unused-vars': ['warn', { argsIgnorePattern: '^_' }],
      'no-console': 'off',
      'no-nonoctal-decimal-escape': 'off',
      'no-empty': ['error', { allowEmptyCatch: true }],
    },
  },
];
