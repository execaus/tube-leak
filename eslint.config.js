// @ts-check
import eslint from '@eslint/js'
import eslintPluginVue from 'eslint-plugin-vue'
import globals from 'globals'
import tseslint from 'typescript-eslint'

export default tseslint.config(
  {
    ignores: ['dist/**', 'src-tauri/**', 'node_modules/**'],
  },
  eslint.configs.recommended,
  ...tseslint.configs.recommended,
  ...eslintPluginVue.configs['flat/recommended'],
  {
    files: ['**/*.vue'],
    languageOptions: {
      parserOptions: {
        parser: tseslint.parser,
      },
    },
  },
  {
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
      },
    },
    rules: {
      'vue/multi-word-component-names': 'off',
      'no-restricted-imports': [
        'error',
        {
          paths: [
            {
              name: '@tauri-apps/api',
              message:
                'Импортируй из подпути пакета (например, "@tauri-apps/api/core" или "@tauri-apps/api/event"), а не из корня. ' +
                'Корневой импорт хуже разбирается сторожем ACL (src-tauri/tests/frontend_acl.rs), который выводит список ' +
                'использованных IPC-команд из импортов, — необходимость выдать разрешение от формы импорта не зависит.',
            },
          ],
        },
      ],
    },
  },
)
