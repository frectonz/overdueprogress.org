import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://overdueprogress.org",
  trailingSlash: "never",
  build: { format: "file" },
  compressHTML: false,
});
