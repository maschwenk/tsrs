import { defineConfig } from "vite-plus";
import { prepareAssets } from "./scripts/prepare-assets.mjs";
import { upstreamWebsite } from "./scripts/upstream-website.mjs";
import { fileURLToPath } from "node:url";

export default defineConfig({
  base: "./",
  resolve: {
    alias: [
      { find: "@tsrs/host", replacement: fileURLToPath(new URL("./src/main.ts", import.meta.url)) },
      {
        find: "@tsrs/page",
        replacement: fileURLToPath(
          new URL("./vendor/packages/typescriptlang-org/src/templates/play.tsx", import.meta.url),
        ),
      },
      {
        find: /^\/fonts\//,
        replacement: fileURLToPath(
          new URL("./vendor/packages/typescriptlang-org/static/fonts/", import.meta.url),
        ),
      },
    ],
  },
  plugins: [
    upstreamWebsite(),
    {
      name: "playground-assets",
      buildStart: prepareAssets,
      generateBundle(_options, bundle) {
        for (const artifact of Object.values(bundle)) {
          if (artifact.type !== "chunk") continue;
          for (const id of artifact.moduleIds) {
            if (
              /\/typescript\/lib\/(?:typescript|tsserverlibrary)\.js/.test(id) ||
              id.includes("/vs/language/typescript/")
            )
              throw new Error(`JavaScript TypeScript runtime must not be bundled: ${id}`);
          }
        }
      },
    },
  ],
  worker: { format: "es" },
  build: { assetsInlineLimit: 0 },
  lint: {
    ignorePatterns: ["vendor/**", "public/**", "src/generated/**"],
    options: { typeAware: true, typeCheck: true },
  },
  fmt: { ignorePatterns: ["vendor/**", "public/**", "src/generated/**"] },
  test: { include: ["src/**/*.test.ts"], testTimeout: 30_000 },
});
