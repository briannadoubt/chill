import eslint from "@eslint/js";
import hooks from "eslint-plugin-react-hooks";
import { defineConfig, globalIgnores } from "eslint/config";
import tseslint from "typescript-eslint";

export default defineConfig([
  globalIgnores(["dist/**", "node_modules/**", ".git/**"]),
  eslint.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    plugins: { "react-hooks": hooks },
    rules: hooks.configs.flat.recommended.rules,
  },
  {
    files: ["vite.config.ts"],
    languageOptions: { globals: { process: "readonly" } },
  },
  {
    files: ["scripts/**/*.mjs"],
    languageOptions: { globals: { URL: "readonly" } },
  },
  {
    files: ["sites-worker.js"],
    languageOptions: {
      globals: {
        Headers: "readonly",
        Request: "readonly",
        Response: "readonly",
        TextEncoder: "readonly",
        URL: "readonly",
        fetch: "readonly",
      },
    },
  },
]);
