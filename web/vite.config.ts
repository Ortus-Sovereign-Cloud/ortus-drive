import { copyFileSync } from "node:fs";
import { defineConfig } from "vite";

// The app's client, embedded in the crate (see ../src/web.rs) and served
// by the core under /assets/apps/drive/. "ortus-kit" stays outside the
// bundle: the browser loads the core's one copy from /assets/kit.js.
export default defineConfig({
  plugins: [
    {
      // pdf.js draws PDFs in a worker of its own, served beside app.js.
      name: "pdf-worker",
      closeBundle() {
        copyFileSync("node_modules/pdfjs-dist/build/pdf.worker.min.mjs", "dist/pdf.worker.min.js");
      },
    },
  ],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
    target: "es2022",
    lib: { entry: "src/app.ts", formats: ["es"], fileName: () => "app.js" },
    rollupOptions: {
      external: ["ortus-kit"],
      output: { paths: { "ortus-kit": "/assets/kit.js" }, chunkFileNames: "[name]-[hash].js" },
    },
  },
});
